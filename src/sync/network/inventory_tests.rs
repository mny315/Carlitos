use super::*;
use crate::sync::random_hex;

fn catalog(id: crate::library::Id, key: &str) -> BTreeMap<crate::library::Id, String> {
    [(id, key.into())].into()
}

fn wait(rx: &mpsc::Receiver<Command>, predicate: impl Fn(&Command) -> bool) -> Command {
    super::tests::wait(rx, |command| {
        if let Command::SyncIncoming(records, rates) = command {
            assert!(
                records.is_empty() && rates.is_empty(),
                "Paused inventory check applied progress"
            );
        }
        predicate(command)
    })
}

#[test]
fn paused_devices_update_book_matches_after_deletion_on_either_side_without_applying_progress()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let (a_tx, a_rx) = mpsc::channel();
    let (b_tx, b_rx) = mpsc::channel();
    let a = Network::without_discovery(temp.path().join("a.json"), random_hex(16)?, a_tx)?;
    let b = Network::without_discovery(temp.path().join("b.json"), random_hex(16)?, b_tx)?;
    let key = "ab".repeat(32);
    // Neither book has ever been played. Local IDs differ across devices.
    a.publish_catalog(vec![], vec![], catalog(1, &key));
    b.publish_catalog(vec![], vec![], catalog(99, &key));
    assert!(a.shared.lock().unwrap().status().ready_books.is_empty());
    b.request(Request::Pair(super::tests::invitation(&a, &a_rx)));
    wait(
        &a_rx,
        |c| matches!(c, Command::SyncStatus(s) if s.ready_books == [1]),
    );
    wait(
        &b_rx,
        |c| matches!(c, Command::SyncStatus(s) if s.ready_books == [99]),
    );
    assert!(!a.shared.lock().unwrap().running && !b.shared.lock().unwrap().running);
    for _ in a_rx.try_iter().chain(b_rx.try_iter()) {}
    let record = Record {
        book: key.clone(),
        part: "cd".repeat(32),
        occurrence: 0,
        position: 9000,
        completed: false,
        reset: false,
        updated: crate::library::now_millis(),
        device: a.shared.lock().unwrap().config.id.clone(),
    };
    a.publish_catalog(vec![record], vec![], catalog(1, &key));
    b.publish_catalog(vec![], vec![], BTreeMap::new());
    wait(
        &a_rx,
        |c| matches!(c, Command::SyncStatus(s) if s.ready_books.is_empty() && !s.peers.is_empty()),
    );
    wait(
        &b_rx,
        |c| matches!(c, Command::SyncStatus(s) if s.ready_books.is_empty() && !s.peers.is_empty()),
    );
    b.publish_catalog(vec![], vec![], catalog(99, &key));
    wait(
        &a_rx,
        |c| matches!(c, Command::SyncStatus(s) if s.ready_books == [1]),
    );
    wait(
        &b_rx,
        |c| matches!(c, Command::SyncStatus(s) if s.ready_books == [99]),
    );
    a.publish_catalog(vec![], vec![], BTreeMap::new());
    wait(
        &b_rx,
        |c| matches!(c, Command::SyncStatus(s) if s.ready_books.is_empty()),
    );
    for command in a_rx.try_iter().chain(b_rx.try_iter()) {
        if let Command::SyncIncoming(records, rates) = command {
            assert!(
                records.is_empty() && rates.is_empty(),
                "Paused inventory check applied progress"
            );
        }
    }
    Ok(())
}

#[test]
fn old_unknown_inventories_offline_devices_and_unpairing_do_not_leave_ready_marks() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (tx, rx) = mpsc::channel();
    let network = Network::without_discovery(temp.path().join("sync.json"), random_hex(16)?, tx)?;
    let peer_id = random_hex(16)?;
    let key = "ef".repeat(32);
    network.publish_catalog(vec![], vec![], catalog(7, &key));
    {
        let mut state = network.shared.lock().unwrap();
        state.remember(Peer {
            id: peer_id.clone(),
            name: "Other device".into(),
            key: random_hex(32)?,
            addresses: vec![],
        })?;
        state.receive_inventory(&peer_id, Some(vec![key.clone()]));
        assert_eq!(state.status().ready_books, [7]);
        state.receive_inventory(&peer_id, None);
        assert!(state.status().ready_books.is_empty());
        state.receive_inventory(&peer_id, Some(vec![key.clone()]));
        state.inventories.get_mut(&peer_id).unwrap().1 = Instant::now() - Duration::from_secs(16);
        assert!(state.status().ready_books.is_empty());
        state.receive_inventory(&peer_id, Some(vec![key]));
    }
    for _ in rx.try_iter() {}
    network.request(Request::Forget(peer_id));
    wait(
        &rx,
        |c| matches!(c, Command::SyncStatus(s) if s.peers.is_empty() && s.ready_books.is_empty()),
    );
    Ok(())
}
