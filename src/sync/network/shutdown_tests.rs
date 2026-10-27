use super::*;

#[test]
fn shutdown_interrupts_idle_incoming_and_outgoing_handshakes() -> Result<()> {
    for outgoing in [false, true] {
        let temp = tempfile::tempdir()?;
        let network = Network::without_discovery(
            temp.path().join("sync.json"),
            super::super::random_hex(16)?,
            mpsc::channel().0,
        )?;
        let shared = network.shared.clone();
        let peer = if outgoing {
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
        let deadline = Instant::now() + Duration::from_secs(3);
        while shared.lock().unwrap().connections.is_empty() {
            ensure!(Instant::now() < deadline, "Handshake worker did not start");
            thread::sleep(Duration::from_millis(2));
        }
        // The peer stays connected but never answers. Drop must wake the
        // blocked reader, rather than wait for the protocol's normal timeout.
        let (finished, received) = mpsc::channel();
        let worker = thread::spawn(move || {
            drop(network);
            let _ = finished.send(());
        });
        let result = received.recv_timeout(Duration::from_secs(1));
        // Always release/join the old implementation before asserting red.
        let _ = peer.shutdown(Shutdown::Both);
        worker.join().unwrap();
        result
            .with_context(|| format!("Shutdown waited for an idle peer (outgoing={outgoing})"))?;
        assert!(shared.lock().unwrap().connections.is_empty());
    }
    Ok(())
}
