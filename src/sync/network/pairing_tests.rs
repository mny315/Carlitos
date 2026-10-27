use super::*;
use crate::sync::random_hex;

fn wait_for(mut ready: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(3);
    while !ready() {
        assert!(Instant::now() < until, "Pairing event before deadline");
        thread::sleep(Duration::from_millis(10));
    }
}

fn unavailable_address() -> Result<SocketAddr> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let address = listener.local_addr()?;
    drop(listener);
    Ok(address)
}

#[test]
fn failed_outgoing_qr_connection_waits_for_successful_reverse_connection() -> Result<()> {
    for already_paired in [false, true] {
        reverse_pairing(already_paired)?;
    }
    Ok(())
}

fn reverse_pairing(already_paired: bool) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (a_tx, a_rx) = mpsc::channel();
    let (b_tx, b_rx) = mpsc::channel();
    let a = Network::without_discovery(temp.path().join("a.json"), random_hex(16)?, a_tx.clone())?;
    let b = Network::without_discovery(temp.path().join("b.json"), random_hex(16)?, b_tx)?;
    let mut qr = Invitation::parse(&super::tests::invitation(&a, &a_rx))?;
    if already_paired {
        let secret = random_hex(32)?;
        for (owner, remote) in [(&a, &b), (&b, &a)] {
            let state = remote.shared.lock().unwrap();
            let peer = Peer {
                id: state.config.id.clone(),
                name: state.config.name.clone(),
                key: secret.clone(),
                addresses: vec![SocketAddr::from((Ipv4Addr::LOCALHOST, state.config.port))],
            };
            drop(state);
            owner.shared.lock().unwrap().remember(peer)?;
        }
    }
    // A firewall blocks scanner -> QR owner, while the other direction works.
    qr.addresses = vec![unavailable_address()?];
    b.request(Request::Pair(qr.encode()?));
    wait_for(|| b.shared.lock().unwrap().pairing_failure.is_some());
    thread::sleep(Duration::from_millis(350));
    for command in b_rx.try_iter() {
        if let Command::SyncStatus(status) = command {
            assert!(!status.error, "Premature pairing error: {}", status.message);
        }
    }
    let state = b.shared.lock().unwrap();
    assert!(state.status().busy);
    let id = state.config.id.clone();
    let ad = Advertisement {
        addresses: vec![SocketAddr::from((Ipv4Addr::LOCALHOST, state.config.port))],
        request: Some((qr.device.clone(), qr.ticket.clone())),
        sync_request: String::new(),
        offer: String::new(),
    };
    drop(state);
    let (peer, ticket) = reverse_peer(&a.shared.lock().unwrap(), &id, &ad).unwrap();
    connect(&a.shared, &a_tx, peer, &ticket, 0)?;
    super::tests::wait(
        &b_rx,
        |command| matches!(command, Command::SyncStatus(status) if status.peers.len() == 1 && !status.busy),
    );
    let state = b.shared.lock().unwrap();
    assert!(state.pairing.is_none());
    assert!(state.pairing_failure.is_none());
    assert!(!state.error);
    assert!(state.message.is_empty());
    assert_eq!(state.config.peers[0].key, qr.key);
    Ok(())
}

#[test]
fn pairing_reports_error_only_when_both_directions_have_expired() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (tx, rx) = mpsc::channel();
    let network = Network::without_discovery(temp.path().join("sync.json"), random_hex(16)?, tx)?;
    let qr = Invitation::new(random_hex(16)?, vec![unavailable_address()?])?;
    network.request(Request::Pair(qr.encode()?));
    wait_for(|| network.shared.lock().unwrap().pairing_failure.is_some());
    for command in rx.try_iter() {
        if let Command::SyncStatus(status) = command {
            assert!(!status.error, "Premature pairing error: {}", status.message);
        }
    }
    {
        let mut state = network.shared.lock().unwrap();
        assert!(state.status().busy);
        state.pairing.as_mut().unwrap().1 = Instant::now();
    }
    network.thread.as_ref().unwrap().thread().unpark();
    wait_for(|| network.shared.lock().unwrap().error);
    let state = network.shared.lock().unwrap();
    assert!(state.pairing.is_none());
    assert!(state.pairing_failure.is_none());
    assert!(!state.status().busy);
    assert!(state.config.peers.is_empty());
    assert!(
        state.message.contains("Device is unavailable")
            || state.message.contains("Устройство недоступно")
    );
    let hello = Hello {
        version: 1,
        from: qr.device,
        to: state.config.id.clone(),
        invitation: qr.ticket,
    };
    assert!(pending_secret(&state, &hello).is_none());
    Ok(())
}
