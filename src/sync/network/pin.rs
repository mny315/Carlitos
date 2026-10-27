use super::*;

const MAX_ATTEMPTS: u8 = 5;
type Candidate = (String, Advertisement);

pub(super) fn parse(text: &str) -> Result<String> {
    let code = text.trim();
    ensure!(
        code.len() == 4 && code.bytes().all(|c| c.is_ascii_digit()),
        "{}",
        crate::app::text(
            "Введите 4 цифры кода устройства",
            "Enter the device's 4-digit code"
        )
    );
    Ok(code.into())
}

pub(super) fn candidates(
    shared: &Arc<Mutex<Shared>>,
    discovered: &BTreeMap<String, Advertisement>,
) -> Result<Vec<Candidate>> {
    let state = shared.lock().unwrap();
    let candidates: Vec<_> = discovered
        .iter()
        .filter(|(id, ad)| {
            **id != state.config.id && !ad.offer.is_empty() && !ad.addresses.is_empty()
        })
        .take(8)
        .map(|(id, ad)| (id.clone(), ad.clone()))
        .collect();
    ensure!(
        !candidates.is_empty(),
        "{}",
        crate::app::text(
            "Устройство не найдено. Подключите устройства к одному Wi-Fi и откройте код на другом устройстве. Если поиск в сети заблокирован, используйте QR.",
            "Device not found. Connect both devices to the same Wi-Fi and show the code on the other device. If network discovery is blocked, use QR."
        )
    );
    Ok(candidates)
}

pub(super) fn pair(
    shared: &Arc<Mutex<Shared>>,
    tx: &mpsc::Sender<Command>,
    pin: &str,
    candidates: Vec<Candidate>,
    generation: u64,
) -> Result<()> {
    let (from, snapshot) = {
        let state = shared.lock().unwrap();
        ensure!(
            !state.stopped && state.generation == generation,
            "Sync cancelled"
        );
        (state.config.id.clone(), state.exchange())
    };
    let until = Instant::now() + Duration::from_secs(10);
    for (device, advertisement) in candidates {
        let hello = Hello {
            version: 2,
            from: from.clone(),
            to: device.clone(),
            invitation: advertisement.offer,
        };
        for address in advertisement
            .addresses
            .iter()
            .filter(|a| valid_address(a))
            .take(8)
        {
            {
                let state = shared.lock().unwrap();
                ensure!(
                    !state.stopped && state.generation == generation,
                    "Sync cancelled"
                );
            }
            if Instant::now() >= until {
                break;
            }
            let Ok(stream) = TcpStream::connect_timeout(address, Duration::from_millis(500)) else {
                continue;
            };
            let _guard = ConnectionGuard::track(shared, &stream)?;
            ensure!(
                shared.lock().unwrap().generation == generation,
                "Sync cancelled"
            );
            // PAKE prevents a recorded exchange from revealing a four-digit PIN.
            // Noise confirms its session key before the QR secret is released.
            let mut channel = match Channel::pin_client(stream, pin, &hello, &snapshot) {
                Ok(channel) => channel,
                // Try another device, not every address of the same device:
                // one entered code must spend at most one guess per invitation.
                Err(_) => break,
            };
            let invitation = channel
                .receive_invitation()
                .and_then(|text| Invitation::parse(&text));
            let Ok(mut invitation) = invitation else {
                break;
            };
            ensure!(
                invitation.device == device && invitation.ticket == hello.invitation,
                "Wrong pairing invitation"
            );
            invitation.addresses.retain(|a| a != address);
            invitation.addresses.insert(0, *address);
            invitation.addresses.truncate(8);
            let mut state = shared.lock().unwrap();
            ensure!(
                !state.stopped && state.generation == generation,
                "Sync cancelled"
            );
            ensure!(
                state.config.peers.len() < MAX_PEERS,
                "Too many paired devices"
            );
            begin_pairing(&mut state, invitation.clone());
            drop(state);
            let peer = Peer {
                id: device,
                name: String::new(),
                key: invitation.key,
                addresses: invitation.addresses,
            };
            connect_pairing(shared, tx, peer, &invitation.ticket, generation);
            return Ok(());
        }
    }
    anyhow::bail!(
        "{}",
        crate::app::text(
            "Не удалось подключиться по коду. Проверьте 4 цифры, общий Wi-Fi и срок действия кода.",
            "Could not pair using this code. Check the 4 digits, shared Wi-Fi and code expiry."
        )
    )
}

pub(super) fn accept(
    stream: TcpStream,
    hello: &Hello,
    bytes: &[u8],
    shared: &Arc<Mutex<Shared>>,
) -> Result<()> {
    let (pin, generation, snapshot) = {
        let mut state = shared.lock().unwrap();
        ensure!(
            !state.stopped && hello.to == state.config.id,
            "Wrong sync device"
        );
        ensure!(state.pin_attempts < MAX_ATTEMPTS, "Too many PIN attempts");
        let (invitation, until) = state
            .invitation
            .as_ref()
            .context("Pairing code is closed")?;
        ensure!(
            invitation.ticket == hello.invitation
                && Instant::now() < *until
                && !invitation.pin.is_empty(),
            "Pairing code expired"
        );
        let pin = invitation.pin.clone();
        // Count before starting crypto, under the lock, including concurrent guesses.
        state.pin_attempts += 1;
        (pin, state.generation, state.exchange())
    };
    let result = (|| -> Result<()> {
        let mut channel = Channel::pin_server(stream, &pin, bytes, &snapshot)?;
        let invitation = {
            let mut state = shared.lock().unwrap();
            ensure!(
                !state.stopped && state.generation == generation,
                "Sync cancelled"
            );
            let (invitation, until) = state
                .invitation
                .as_mut()
                .context("Pairing code is closed")?;
            ensure!(
                invitation.ticket == hello.invitation
                    && invitation.pin == pin
                    && Instant::now() < *until,
                "Pairing code expired"
            );
            let encoded = invitation.encode()?;
            // Only one authenticated bootstrap can receive the invitation.
            invitation.pin.clear();
            encoded
        };
        channel.send_invitation(&invitation)
    })();
    if result.is_err() {
        let mut state = shared.lock().unwrap();
        if state.pin_attempts >= MAX_ATTEMPTS
            && state
                .invitation
                .as_ref()
                .is_some_and(|(i, _)| i.ticket == hello.invitation && !i.pin.is_empty())
        {
            state.invitation = None;
            state.message = crate::app::text(
                "Слишком много неверных попыток. Покажите новый код подключения.",
                "Too many incorrect attempts. Show a new pairing code.",
            )
            .into();
            state.error = true;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_digits_preserve_leading_zeroes_and_reject_other_inputs() {
        assert_eq!(parse(" 0042\n").unwrap(), "0042");
        for invalid in ["", "123", "12345", "12a4", "12 34", "１２３４"] {
            assert!(parse(invalid).is_err());
        }
    }
}
