use super::*;
use std::io::{ErrorKind, Read};

#[test]
fn shutdown_closes_idle_peers_and_joins_incoming_and_outgoing_workers() -> Result<()> {
    for outgoing in [false, true] {
        let temp = tempfile::tempdir()?;
        let network = Network::without_discovery(
            temp.path().join("sync.json"),
            super::super::random_hex(16)?,
            mpsc::channel().0,
        )?;
        let shared = network.shared.clone();
        let mut peer = if outgoing {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            listener.set_nonblocking(true)?;
            let invitation =
                Invitation::new(super::super::random_hex(16)?, vec![listener.local_addr()?])?;
            network.request(Request::Pair(invitation.encode()?));
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error.into()),
                }
                ensure!(
                    Instant::now() < deadline,
                    "Outgoing connection did not start"
                );
                thread::sleep(Duration::from_millis(2));
            }
        } else {
            network.request(Request::Running(true));
            let port = shared.lock().unwrap().config.port;
            TcpStream::connect((Ipv4Addr::LOCALHOST, port))?
        };
        // Accepted sockets inherit the listener's nonblocking mode on Windows.
        peer.set_nonblocking(false)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        while shared.lock().unwrap().connections.is_empty() {
            ensure!(Instant::now() < deadline, "Handshake worker did not start");
            thread::sleep(Duration::from_millis(2));
        }
        // The peer never answers. Drop must promptly close its remote end.
        // Keep this peer's send side open until the worker has finished.
        let (finished, received) = mpsc::channel();
        let (starting, started) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = starting.send(());
            let started = Instant::now();
            drop(network);
            let _ = finished.send(started.elapsed());
        });
        started.recv_timeout(Duration::from_secs(5))?;
        let closed = (|| -> std::io::Result<()> {
            let until = Instant::now() + Duration::from_secs(1);
            let mut bytes = [0; 1024];
            loop {
                peer.set_read_timeout(Some(until.saturating_duration_since(Instant::now())))?;
                match peer.read(&mut bytes) {
                    Ok(0) => return Ok(()),
                    // Outgoing handshakes may have already sent their hello.
                    Ok(_) => {}
                    Err(error)
                        if matches!(
                            error.kind(),
                            ErrorKind::ConnectionReset
                                | ErrorKind::ConnectionAborted
                                | ErrorKind::NotConnected
                        ) =>
                    {
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                }
            }
        })();
        let result = received.recv_timeout(Duration::from_secs(5));
        // Always release/join the old implementation before asserting red.
        let _ = peer.shutdown(Shutdown::Both);
        worker.join().unwrap();
        closed.with_context(|| format!("Shutdown left an idle peer open (outgoing={outgoing})"))?;
        let elapsed = result
            .with_context(|| format!("Shutdown waited for an idle peer (outgoing={outgoing})"))?;
        // Winsock can finish an already pending recv at its 2 s I/O timeout
        // even after shutdown closes the peer's connection. Unix wakes it.
        let limit = Duration::from_secs(if cfg!(windows) { 3 } else { 1 });
        ensure!(
            elapsed < limit,
            "Shutdown took {elapsed:?} for an idle peer (outgoing={outgoing})"
        );
        assert!(shared.lock().unwrap().connections.is_empty());
    }
    Ok(())
}
