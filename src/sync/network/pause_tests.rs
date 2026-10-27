use super::*;
use crate::sync::random_hex;

fn until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !ready() {
        assert!(Instant::now() < deadline, "Sync transition timed out");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn paused_devices_make_no_connections_and_keep_the_last_catalog_until_resumed() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (a_tx, a_rx) = mpsc::channel();
    let (b_tx, b_rx) = mpsc::channel();
    let a = Network::without_discovery(temp.path().join("a.json"), random_hex(16)?, a_tx)?;
    let b = Network::without_discovery(temp.path().join("b.json"), random_hex(16)?, b_tx)?;
    let key = "ab".repeat(32);
    a.publish_catalog(vec![], vec![], [(1, key.clone())].into());
    b.publish_catalog(vec![], vec![], [(99, key.clone())].into());
    b.request(Request::Pair(super::tests::invitation(&a, &a_rx)));
    until(|| b.shared.lock().unwrap().status().ready_books == [99]);
    until(|| a.shared.lock().unwrap().status().ready_books == [1]);
    a.request(Request::Running(false));
    b.request(Request::Running(false));
    until(|| {
        a.shared.lock().unwrap().connections.is_empty()
            && b.shared.lock().unwrap().connections.is_empty()
    });
    let before = (
        a.shared.lock().unwrap().next_connection,
        b.shared.lock().unwrap().next_connection,
    );
    for _ in a_rx.try_iter().chain(b_rx.try_iter()) {}
    // Local deletion must not inform the paused peer. Wait longer than the
    // former three-second background inventory timer, then expire its cache.
    a.publish_catalog(vec![], vec![], BTreeMap::new());
    thread::sleep(Duration::from_millis(3400));
    assert_eq!(a.shared.lock().unwrap().next_connection, before.0);
    assert_eq!(b.shared.lock().unwrap().next_connection, before.1);
    assert!(
        a_rx.try_iter()
            .chain(b_rx.try_iter())
            .all(|c| !matches!(c, Command::SyncIncoming(..)))
    );
    {
        let mut state = b.shared.lock().unwrap();
        for (_, updated) in state.inventories.values_mut() {
            *updated = Instant::now() - Duration::from_secs(20);
        }
        assert_eq!(state.status().ready_books, [99]);
    }
    a.request(Request::Running(true));
    b.request(Request::Running(true));
    until(|| b.shared.lock().unwrap().status().ready_books.is_empty());
    // Resuming must actually replace the cached catalog, not merely expire it.
    let a_id = a.shared.lock().unwrap().config.id.clone();
    until(|| {
        b.shared
            .lock()
            .unwrap()
            .inventories
            .get(&a_id)
            .is_some_and(|(books, at)| books.is_empty() && at.elapsed() < Duration::from_secs(2))
    });
    Ok(())
}

#[test]
fn enabled_peer_cannot_send_progress_or_inventory_to_a_paused_device() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (a_tx, a_rx) = mpsc::channel();
    let (b_tx, b_rx) = mpsc::channel();
    let a = Network::without_discovery(temp.path().join("a.json"), random_hex(16)?, a_tx)?;
    let b = Network::without_discovery(temp.path().join("b.json"), random_hex(16)?, b_tx.clone())?;
    b.request(Request::Pair(super::tests::invitation(&a, &a_rx)));
    until(|| b.shared.lock().unwrap().config.peers.len() == 1);
    until(|| {
        a.shared.lock().unwrap().connections.is_empty()
            && b.shared.lock().unwrap().connections.is_empty()
    });
    for _ in a_rx.try_iter() {}
    let record = super::super::compact::tests::fixture().records.remove(0);
    b.publish(vec![record], vec![]);
    let (peer, generation) = {
        let mut state = b.shared.lock().unwrap();
        // Exercise the real encrypted connection directly without concurrent retries.
        state.manual = Some(Instant::now());
        (state.config.peers[0].clone(), state.generation)
    };
    assert!(connect(&b.shared, &b_tx, peer, "", generation).is_err());
    assert!(
        a_rx.try_iter()
            .all(|c| !matches!(c, Command::SyncIncoming(..)))
    );
    assert!(
        a.shared
            .lock()
            .unwrap()
            .inventories
            .values()
            .all(|(books, _)| books.is_empty())
    );
    for _ in b_rx.try_iter() {}
    Ok(())
}

#[test]
fn pausing_interrupts_an_inflight_handshake_and_invalidates_queued_data() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (tx, _rx) = mpsc::channel();
    let network = Network::without_discovery(temp.path().join("sync.json"), random_hex(16)?, tx)?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let invitation = Invitation::new(random_hex(16)?, vec![listener.local_addr()?])?;
    network.request(Request::Pair(invitation.encode()?));
    listener.set_nonblocking(true)?;
    let mut peer = None;
    until(|| {
        peer = listener.accept().ok().map(|(s, _)| s);
        peer.is_some()
    });
    until(|| !network.shared.lock().unwrap().connections.is_empty());
    let generation = network.shared.lock().unwrap().generation;
    network.request(Request::Running(false));
    assert!(!network.accepts(generation));
    until(|| network.shared.lock().unwrap().connections.is_empty());
    assert!(!network.active());
    assert!(network.shared.lock().unwrap().pairing.is_none());
    Ok(())
}

#[test]
fn network_discovery_is_idle_until_explicitly_started_and_stops_when_paused() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let network = Network::start(
        temp.path().join("sync.json"),
        random_hex(16)?,
        mpsc::channel().0,
    )?;
    thread::sleep(Duration::from_millis(150));
    assert!(!network.shared.lock().unwrap().discovering);
    network.request(Request::Running(true));
    until(|| network.shared.lock().unwrap().discovering);
    network.request(Request::Running(false));
    until(|| !network.shared.lock().unwrap().discovering);
    assert!(!network.active());
    Ok(())
}
