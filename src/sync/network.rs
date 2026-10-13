mod pin;

use super::invitation::{Invitation, valid_address};
use super::protocol::{self, Channel, Exchange, Hello};
use super::{PeerStatus, RateRecord, Record, Status, valid_hex};
use crate::app::Command;
use anyhow::{Context, Result, ensure};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const SERVICE: &str = "_carlitos._tcp.local.";
const MAX_PEERS: usize = 16;

pub enum Request {
    Invite,
    CancelInvite,
    Pair(String),
    Forget(String),
    Now,
    Refresh,
}

#[derive(Clone, Serialize, Deserialize)]
struct Peer {
    id: String,
    name: String,
    key: String,
    addresses: Vec<SocketAddr>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Config {
    version: u32,
    id: String,
    name: String,
    port: u16,
    peers: Vec<Peer>,
}
impl Config {
    fn load(path: &Path, id: String) -> Result<Self> {
        let mut config = match std::fs::read(path) {
            Ok(bytes) => {
                serde_json::from_slice::<Self>(&bytes).context("Cannot read saved sync devices")?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self {
                version: 1,
                name: crate::platform::device_name(),
                id: id.clone(),
                port: 0,
                peers: vec![],
            },
            Err(e) => return Err(e.into()),
        };
        ensure!(
            config.version == 1 && config.id == id,
            "Sync configuration belongs to a different library"
        );
        ensure!(config.peers.len() <= MAX_PEERS, "Too many paired devices");
        for p in &config.peers {
            ensure!(
                valid_hex(&p.id, 16) && valid_hex(&p.key, 32),
                "Invalid saved device"
            );
        }
        config.name = crate::platform::device_name();
        Ok(config)
    }
    fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("Sync directory missing")?;
        std::fs::create_dir_all(parent)?;
        // NamedTempFile is owner-only on Unix. Keys never appear in settings,
        // mDNS announcements, logs, or unencrypted progress messages.
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.as_file().sync_all()?;
        file.persist(path)?;
        #[cfg(unix)]
        std::fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
}

fn local_addresses(port: u16) -> Vec<SocketAddr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|a| match a.ip() {
            std::net::IpAddr::V4(ip) if !ip.is_loopback() => Some(SocketAddr::from((ip, port))),
            _ => None,
        })
        .filter(valid_address)
        .take(8)
        .collect()
}

fn merge_addresses(saved: &[SocketAddr], discovered: &[SocketAddr]) -> Vec<SocketAddr> {
    let mut addresses = Vec::new();
    for address in saved.iter().take(1).chain(discovered).chain(saved) {
        if valid_address(address) && !addresses.contains(address) {
            addresses.push(*address);
            if addresses.len() == 8 {
                break;
            }
        }
    }
    addresses
}

#[derive(Clone)]
struct Advertisement {
    addresses: Vec<SocketAddr>,
    request: Option<(String, String)>,
    sync_request: String,
    offer: String,
}

fn service_info(
    config: &Config,
    request: Option<&Invitation>,
    offer: Option<&Invitation>,
    sync_request: u64,
) -> Result<ServiceInfo> {
    let sync_request = sync_request.to_string();
    let mut properties = vec![
        ("id", config.id.as_str()),
        ("v", "1"),
        ("sync", sync_request.as_str()),
    ];
    if let Some(request) = request {
        // Only the public ticket is advertised. The QR secret stays on the two
        // devices and is proved by the encrypted handshake in either direction.
        properties.push(("pair-to", &request.device));
        properties.push(("pair-ticket", &request.ticket));
    }
    if let Some(offer) = offer.filter(|i| !i.pin.is_empty()) {
        properties.push(("pair-pin", &offer.ticket));
    }
    Ok(ServiceInfo::new(
        SERVICE,
        &config.id,
        &format!("carlitos-{}.local.", config.id),
        "",
        config.port,
        &properties[..],
    )?
    .enable_addr_auto())
}

struct Shared {
    config: Config,
    path: PathBuf,
    invitation: Option<(Invitation, Instant)>,
    pin_attempts: u8,
    pairing: Option<(Invitation, Instant)>,
    records: Vec<Record>,
    rates: Vec<RateRecord>,
    online: BTreeMap<String, Instant>,
    message: String,
    error: bool,
    manual: Option<Instant>,
    activity: Option<Instant>,
    sync_request: u64,
    stopped: bool,
    dirty: bool,
    generation: u64,
    sequence: u64,
    delivered: BTreeMap<String, u64>,
}
impl Shared {
    fn exchange(&self) -> Exchange {
        Exchange {
            name: self.config.name.clone(),
            port: self.config.port,
            records: self.records.clone(),
            rates: self.rates.clone(),
        }
    }
    fn status(&self) -> Status {
        Status {
            enabled: true,
            busy: self.manual.is_some()
                || self
                    .activity
                    .is_some_and(|t| t.elapsed() < Duration::from_millis(600)),
            error: self.error,
            name: self.config.name.clone(),
            invitation: self
                .invitation
                .as_ref()
                .and_then(|(i, _)| i.encode().ok())
                .unwrap_or_default(),
            code: self
                .invitation
                .as_ref()
                .map(|(i, _)| i.code())
                .unwrap_or_default(),
            peers: self
                .config
                .peers
                .iter()
                .map(|p| PeerStatus {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    online: self
                        .online
                        .get(&p.id)
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(15)),
                })
                .collect(),
            message: self.message.clone(),
        }
    }
    fn remember(&mut self, peer: Peer) -> Result<()> {
        let mut config = self.config.clone();
        config.peers.retain(|p| p.id != peer.id);
        ensure!(config.peers.len() < MAX_PEERS, "Too many paired devices");
        config.peers.push(peer);
        config.save(&self.path)?;
        self.config = config;
        Ok(())
    }
}

pub struct Network {
    shared: Arc<Mutex<Shared>>,
    requests: mpsc::Sender<Request>,
    thread: Option<JoinHandle<()>>,
}
impl Network {
    pub fn start(path: PathBuf, id: String, tx: mpsc::Sender<Command>) -> Result<Self> {
        Self::start_inner(path, id, tx, true)
    }
    fn start_inner(
        path: PathBuf,
        id: String,
        tx: mpsc::Sender<Command>,
        discovery: bool,
    ) -> Result<Self> {
        let mut config = Config::load(&path, id)?;
        let listener = listen(config.port).or_else(|_| listen(0))?;
        listener.set_nonblocking(true)?;
        config.port = listener.local_addr()?.port();
        config.save(&path)?;
        let mdns = if discovery {
            let daemon = ServiceDaemon::new()?;
            daemon.disable_interface(mdns_sd::IfKind::IPv6)?;
            let info = service_info(&config, None, None, 0)?;
            if let Err(error) = daemon.register(info) {
                let _ = daemon.shutdown();
                return Err(error.into());
            }
            let receiver = match daemon.browse(SERVICE) {
                Ok(r) => r,
                Err(e) => {
                    let _ = daemon.shutdown();
                    return Err(e.into());
                }
            };
            Some((daemon, receiver))
        } else {
            None
        };
        let shared = Arc::new(Mutex::new(Shared {
            config,
            path,
            invitation: None,
            pin_attempts: 0,
            pairing: None,
            records: vec![],
            rates: vec![],
            online: BTreeMap::new(),
            message: String::new(),
            error: false,
            manual: None,
            activity: None,
            sync_request: 0,
            stopped: false,
            dirty: true,
            generation: 0,
            sequence: 0,
            delivered: BTreeMap::new(),
        }));
        let (requests, rx) = mpsc::channel();
        let worker = shared.clone();
        let thread = thread::Builder::new().name("carlitos-sync".into()).spawn(move || {
            let mut discovered = BTreeMap::<String, Advertisement>::new();
            let mut advertised_request = None;
            let mut advertised_offer = None;
            let mut advertised_sync = 0;
            let mut attempts = BTreeMap::<String, Instant>::new();
            let mut pending = BTreeSet::<String>::new();
            let mut jobs = Vec::<(String, JoinHandle<()>)>::new();
            let mut last_status = None;
            loop {
                if worker.lock().unwrap().stopped { break; }
                jobs.retain_mut(|(_, job)| !job.is_finished());
                if let Some((_, receiver)) = &mdns {
                    while let Ok(event) = receiver.try_recv() {
                        if let ServiceEvent::ServiceResolved(info) = event
                            && info.get_property_val_str("v") == Some("1")
                            && let Some(id) = info.get_property_val_str("id")
                            && valid_hex(id, 16)
                        {
                            let addresses = info.get_addresses_v4().into_iter().map(|ip| SocketAddr::from((ip, info.get_port()))).filter(valid_address).take(8).collect();
                            let request = info.get_property_val_str("pair-to").zip(info.get_property_val_str("pair-ticket"))
                                .filter(|(to, ticket)| valid_hex(to, 16) && valid_hex(ticket, 16))
                                .map(|(to, ticket)| (to.to_owned(), ticket.to_owned()));
                            let sync_request = info.get_property_val_str("sync").unwrap_or("0").to_owned();
                            if sync_request != "0" && discovered.get(id).is_none_or(|ad| ad.sync_request != sync_request)
                                && attempts.get(id).is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
                                pending.insert(id.into());
                            }
                            let offer = info.get_property_val_str("pair-pin").filter(|v| valid_hex(v, 16)).unwrap_or("").to_owned();
                            discovered.insert(id.into(), Advertisement { addresses, request, sync_request, offer });
                            // Bound untrusted discovery announcements.
                            if discovered.len() > 128 { discovered.clear(); }
                        }
                    }
                }
                while let Ok(request) = rx.try_recv() {
                    let result = (|| -> Result<()> {
                        match request {
                            Request::Invite => {
                                let mut state = worker.lock().unwrap();
                                ensure!(state.config.peers.len() < MAX_PEERS, "Too many paired devices");
                                let invitation = Invitation::new(state.config.id.clone(), local_addresses(state.config.port))?;
                                state.invitation = Some((invitation, Instant::now() + Duration::from_secs(300)));
                                state.pin_attempts = 0;
                                state.message.clear();
                                state.error = false;
                            }
                            Request::CancelInvite => { worker.lock().unwrap().invitation = None; }
                            Request::Forget(id) => {
                                let mut state = worker.lock().unwrap();
                                let mut config = state.config.clone();
                                config.peers.retain(|p| p.id != id);
                                config.save(&state.path)?;
                                state.config = config;
                                state.online.remove(&id);
                                if state.pairing.as_ref().is_some_and(|(i, _)| i.device == id) { state.pairing = None; }
                                state.generation += 1;
                            }
                            Request::Now => {
                                let mut state = worker.lock().unwrap();
                                state.dirty = true;
                                state.manual = Some(Instant::now());
                                state.message.clear();
                                state.error = false;
                                state.sync_request += 1;
                            }
                            Request::Refresh => { worker.lock().unwrap().dirty = true; }
                            Request::Pair(text) => {
                                ensure!(jobs.len() < 8, "Sync is busy; try again");
                                #[cfg(target_os = "android")]
                                ensure!(crate::android::bridge::has_local_network()?, "{}", crate::app::text(
                                    "Подключите телефон к тому же Wi-Fi, что и другое устройство, и повторите подключение. Через мобильный интернет локальная синхронизация недоступна.",
                                    "Connect your phone to the same Wi-Fi as the other device and try again. Local sync is unavailable over mobile data."));
                                if !text.trim().to_ascii_lowercase().starts_with("carlitos://") {
                                    let code = pin::parse(&text)?;
                                    let candidates = pin::candidates(&worker, &discovered)?;
                                    let shared = worker.clone();
                                    let sender = tx.clone();
                                    let generation = shared.lock().unwrap().generation;
                                    let job = thread::Builder::new().name("carlitos-pin".into()).spawn(move || {
                                        if let Err(error) = pin::pair(&shared, &sender, &code, candidates, generation) {
                                            let mut state = shared.lock().unwrap();
                                            if !state.stopped && state.generation == generation {
                                                state.message = format!("{error:#}");
                                                state.error = true;
                                            }
                                        }
                                    })?;
                                    jobs.push((String::new(), job));
                                    return Ok(());
                                }
                                let invitation = Invitation::parse(&text)?;
                                let mut state = worker.lock().unwrap();
                                ensure!(invitation.device != state.config.id, "Cannot pair this device with itself");
                                ensure!(state.config.peers.len() < MAX_PEERS, "Too many paired devices");
                                let generation = state.generation;
                                state.pairing = Some((invitation.clone(), Instant::now() + Duration::from_secs(300)));
                                drop(state);
                                let addresses = merge_addresses(&invitation.addresses, discovered.get(&invitation.device).map_or(&[], |ad| ad.addresses.as_slice()));
                                let peer = Peer { id: invitation.device.clone(), name: String::new(), key: invitation.key, addresses };
                                let shared = worker.clone();
                                let sender = tx.clone();
                                let id = peer.id.clone();
                                let job = thread::Builder::new().name("carlitos-pair".into()).spawn(move || {
                                    let peer_id = peer.id.clone();
                                    if let Err(error) = connect(&shared, &sender, peer, &invitation.ticket, generation) {
                                        let mut state = shared.lock().unwrap();
                                        if !state.config.peers.iter().any(|p| p.id == peer_id) {
                                            state.message = format!("{}: {error:#}", crate::app::text("Не удалось связать устройства. Проверьте сеть и обновите QR", "Could not pair devices. Check the network and renew the QR code"));
                                            state.error = true;
                                        }
                                    }
                                })?;
                                jobs.push((id, job));
                            }
                        }
                        Ok(())
                    })();
                    if let Err(error) = result { let mut state = worker.lock().unwrap(); state.message = format!("{error:#}"); state.error = true; }
                }
                while jobs.len() < 8 && let Ok((stream, address)) = listener.accept() {
                    let shared = worker.clone();
                    let sender = tx.clone();
                    if let Ok(job) = thread::Builder::new().name("carlitos-sync-in".into()).spawn(move || { let _ = accept(stream, address, &shared, &sender); }) { jobs.push((String::new(), job)); }
                }
                let (peers, reverse, dirty, generation) = {
                    let mut state = worker.lock().unwrap();
                    if state.invitation.as_ref().is_some_and(|(_, until)| Instant::now() >= *until) { state.invitation = None; }
                    if state.pairing.as_ref().is_some_and(|(_, until)| Instant::now() >= *until) { state.pairing = None; }
                    let request = state.pairing.as_ref().map(|(i, _)| (i.device.clone(), i.ticket.clone()));
                    let offer = state.invitation.as_ref().filter(|(i, _)| !i.pin.is_empty()).map(|(i, _)| i.ticket.clone());
                    if (request != advertised_request || offer != advertised_offer || state.sync_request != advertised_sync) && let Some((daemon, _)) = &mdns {
                        if let Ok(info) = service_info(&state.config, state.pairing.as_ref().map(|(i, _)| i), state.invitation.as_ref().map(|(i, _)| i), state.sync_request) {
                            let _ = daemon.register(info);
                        }
                        advertised_request = request;
                        advertised_offer = offer;
                        advertised_sync = state.sync_request;
                    }
                    if let Some(started) = state.manual {
                        let missing: Vec<_> = state.config.peers.iter().filter(|p| state.online.get(&p.id).is_none_or(|seen| *seen < started)).map(|p| p.name.clone()).collect();
                        if (missing.is_empty() && started.elapsed() >= Duration::from_millis(600)) || started.elapsed() >= Duration::from_secs(8) {
                            state.manual = None;
                            state.error = !missing.is_empty() || state.config.peers.is_empty();
                            state.message = if state.config.peers.is_empty() {
                                crate::app::text("Сначала свяжите устройства в настройках через QR-код", "Pair devices in settings using a QR code first").into()
                            } else if missing.is_empty() {
                                crate::app::text("Синхронизация выполнена", "Synchronization complete").into()
                            } else {
                                format!("{}: {}. {}", crate::app::text("Нет связи", "Unavailable"), missing.join(", "), crate::app::text("Откройте Carlitos на другом устройстве и проверьте общую сеть и доступ через брандмауэр", "Open Carlitos on the other device and check the shared network and firewall access"))
                            };
                            let _ = tx.send(Command::SyncFinished(state.message.clone()));
                        }
                    }
                    let reverse = discovered.iter().filter_map(|(id, ad)| reverse_peer(&state, id, ad)).collect::<Vec<_>>();
                    let status = state.status();
                    if last_status.as_ref() != Some(&status) { let _ = tx.send(Command::SyncStatus(status.clone())); last_status = Some(status); }
                    (state.config.peers.clone(), reverse, std::mem::take(&mut state.dirty), state.generation)
                };
                if dirty { pending.extend(peers.iter().map(|peer| peer.id.clone())); }
                pending.retain(|id| peers.iter().any(|peer| &peer.id == id));
                for (mut peer, ticket) in peers.into_iter().map(|p| (p, String::new())).chain(reverse) {
                    if jobs.len() >= 8 || jobs.iter().any(|(id, _)| *id == peer.id) { continue; }
                    if !pending.contains(&peer.id) && attempts.get(&peer.id).is_some_and(|t| t.elapsed() < Duration::from_secs(3)) { continue; }
                    if let Some(ad) = discovered.get(&peer.id) { peer.addresses = merge_addresses(&peer.addresses, &ad.addresses); }
                    attempts.insert(peer.id.clone(), Instant::now());
                    let shared = worker.clone();
                    let sender = tx.clone();
                    let id = peer.id.clone();
                    if let Ok(job) = thread::Builder::new().name("carlitos-sync-out".into()).spawn(move || { let _ = connect(&shared, &sender, peer, &ticket, generation); }) { pending.remove(&id); jobs.push((id, job)); }
                }
                // Requests/publications unpark immediately; only active exchanges
                // need the shorter polling interval.
                thread::park_timeout(Duration::from_millis(if jobs.is_empty() { 100 } else { 20 }));
            }
            if let Some((daemon, _)) = mdns { let _ = daemon.shutdown(); }
            for (_, job) in jobs { let _ = job.join(); }
        })?;
        Ok(Self {
            shared,
            requests,
            thread: Some(thread),
        })
    }
    pub fn publish(&self, records: Vec<Record>, rates: Vec<RateRecord>) {
        let mut state = self.shared.lock().unwrap();
        if state.records != records || state.rates != rates {
            state.records = records;
            state.rates = rates;
            state.dirty = true;
            state.sequence += 1;
            state.activity = Some(Instant::now());
            if let Some(thread) = &self.thread {
                thread.thread().unpark();
            }
        }
    }
    /// Give reachable peers a bounded opportunity to receive the final checkpoint.
    pub fn flush(&self) {
        let until = Instant::now() + Duration::from_millis(1500);
        self.shared.lock().unwrap().dirty = true;
        loop {
            let state = self.shared.lock().unwrap();
            let pending = state.online.iter().any(|(id, seen)| {
                seen.elapsed() < Duration::from_secs(15)
                    && state.delivered.get(id).copied().unwrap_or(0) < state.sequence
            });
            if !pending || Instant::now() >= until {
                return;
            }
            drop(state);
            thread::sleep(Duration::from_millis(20));
        }
    }
    pub fn request(&self, request: Request) {
        let _ = self.requests.send(request);
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
    }
    #[cfg(feature = "android-playback-tests")]
    pub(crate) fn test_status(&self) -> serde_json::Value {
        let state = self.shared.lock().unwrap();
        serde_json::json!({"message": state.message, "peers": state.config.peers.len(),
            "records": state.records.len(), "port": state.config.port})
    }
}
impl Drop for Network {
    fn drop(&mut self) {
        self.shared.lock().unwrap().stopped = true;
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

fn reverse_peer(state: &Shared, id: &str, ad: &Advertisement) -> Option<(Peer, String)> {
    let (target, ticket) = ad.request.as_ref()?;
    let (invitation, until) = state.invitation.as_ref()?;
    if target != &state.config.id
        || ticket != &invitation.ticket
        || Instant::now() >= *until
        || id == state.config.id
        || state.config.peers.iter().any(|p| p.id == id)
    {
        return None;
    }
    Some((
        Peer {
            id: id.into(),
            name: String::new(),
            key: invitation.key.clone(),
            addresses: ad.addresses.clone(),
        },
        ticket.clone(),
    ))
}

fn finish_pairing(state: &mut Shared, id: &str, ticket: &str, secret: &str) {
    if state
        .invitation
        .as_ref()
        .is_some_and(|(i, _)| i.ticket == ticket && i.key == secret)
    {
        state.invitation = None;
    }
    if state
        .pairing
        .as_ref()
        .is_some_and(|(i, _)| i.device == id && i.key == secret)
    {
        state.pairing = None;
    }
    if !ticket.is_empty() {
        state.message.clear();
        state.error = false;
    }
}

fn pending_secret<'a>(state: &'a Shared, hello: &Hello) -> Option<&'a str> {
    state
        .pairing
        .as_ref()
        .filter(|(i, until)| {
            i.device == hello.from && i.ticket == hello.invitation && Instant::now() < *until
        })
        .map(|(i, _)| i.key.as_str())
}

fn listen(port: u16) -> Result<TcpListener> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    #[cfg(unix)]
    socket.set_reuse_address(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())?;
    socket.listen(16)?;
    Ok(socket.into())
}
fn key(value: &str) -> Result<[u8; 32]> {
    hex::decode(value)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid pairing key"))
}
fn connect(
    shared: &Arc<Mutex<Shared>>,
    tx: &mpsc::Sender<Command>,
    mut peer: Peer,
    invitation: &str,
    generation: u64,
) -> Result<()> {
    let (hello, exchange, sequence) = {
        let state = shared.lock().unwrap();
        ensure!(
            !state.stopped && state.generation == generation,
            "Sync cancelled"
        );
        (
            Hello {
                version: 1,
                from: state.config.id.clone(),
                to: peer.id.clone(),
                invitation: invitation.into(),
            },
            state.exchange(),
            state.sequence,
        )
    };
    let mut connection = None;
    let secret = key(&peer.key)?;
    let mut last_error = anyhow::anyhow!(crate::app::text(
        "Устройство недоступно. Подключите оба устройства к одному Wi-Fi, откройте Carlitos и проверьте, не блокирует ли VPN или брандмауэр локальную сеть",
        "Device is unavailable. Connect both devices to the same Wi-Fi, open Carlitos, and check whether a VPN or firewall blocks the local network"
    ));
    let until = Instant::now() + Duration::from_secs(6);
    for address in peer.addresses.iter().filter(|a| valid_address(a)).take(8) {
        if Instant::now() >= until {
            break;
        }
        if let Ok(stream) = TcpStream::connect_timeout(address, Duration::from_millis(500)) {
            match Channel::client(stream, &secret, &hello, &exchange) {
                Ok(channel) => {
                    connection = Some(channel);
                    // Prefer the last authenticated endpoint over VPN/stale addresses.
                    let address = *address;
                    peer.addresses.retain(|a| *a != address);
                    peer.addresses.insert(0, address);
                    break;
                }
                Err(error) => last_error = error,
            }
        }
    }
    let mut channel = connection.ok_or(last_error)?;
    channel.send(&exchange)?;
    let received = channel.receive()?;
    let mut state = shared.lock().unwrap();
    ensure!(
        !state.stopped && state.generation == generation,
        "Sync cancelled"
    );
    ensure!(
        state
            .config
            .peers
            .iter()
            .any(|p| p.id == peer.id && p.key == peer.key)
            || state
                .invitation
                .as_ref()
                .is_some_and(|(i, until)| i.ticket == invitation
                    && i.key == peer.key
                    && Instant::now() < *until)
            || state
                .pairing
                .as_ref()
                .is_some_and(|(i, until)| i.ticket == invitation
                    && i.key == peer.key
                    && i.device == peer.id
                    && Instant::now() < *until),
        "Pairing cancelled or device unpaired"
    );
    peer.name = received.name;
    if !state.config.peers.iter().any(|p| {
        p.id == peer.id && p.key == peer.key && p.name == peer.name && p.addresses == peer.addresses
    }) {
        state.remember(peer.clone())?;
    }
    state.delivered.insert(peer.id.clone(), sequence);
    finish_pairing(&mut state, &peer.id, invitation, &peer.key);
    state.online.insert(peer.id, Instant::now());
    let _ = tx.send(Command::SyncIncoming(received.records, received.rates));
    Ok(())
}
fn accept(
    mut stream: TcpStream,
    address: SocketAddr,
    shared: &Arc<Mutex<Shared>>,
    tx: &mpsc::Sender<Command>,
) -> Result<()> {
    let (hello, bytes) = protocol::read_hello(&mut stream)?;
    if hello.version == 2 {
        return pin::accept(stream, &hello, &bytes, shared);
    }
    let (secret, generation, snapshot) = {
        let state = shared.lock().unwrap();
        ensure!(
            !state.stopped && hello.to == state.config.id,
            "Wrong sync device"
        );
        let invitation = state
            .invitation
            .as_ref()
            .filter(|(i, until)| i.ticket == hello.invitation && Instant::now() < *until);
        let secret = invitation
            .map(|(i, _)| i.key.clone())
            .or_else(|| pending_secret(&state, &hello).map(str::to_owned))
            .or_else(|| {
                state
                    .config
                    .peers
                    .iter()
                    .find(|p| p.id == hello.from)
                    .map(|p| p.key.clone())
            })
            .context("Device is not paired")?;
        (secret, state.generation, state.exchange())
    };
    let mut channel = Channel::server(stream, &key(&secret)?, &bytes, &snapshot)?;
    let received = channel.receive()?;
    let (exchange, sequence) = {
        let mut state = shared.lock().unwrap();
        ensure!(
            !state.stopped && state.generation == generation,
            "Sync cancelled"
        );
        let paired = state
            .config
            .peers
            .iter()
            .any(|p| p.id == hello.from && p.key == secret);
        if !paired {
            ensure!(
                state
                    .invitation
                    .as_ref()
                    .is_some_and(|(i, until)| i.ticket == hello.invitation
                        && i.key == secret
                        && Instant::now() < *until)
                    || pending_secret(&state, &hello) == Some(secret.as_str()),
                "Pairing QR has expired"
            );
        }
        let endpoint = SocketAddr::new(address.ip(), received.port);
        if !state.config.peers.iter().any(|p| {
            p.id == hello.from
                && p.key == secret
                && p.name == received.name
                && p.addresses.contains(&endpoint)
        }) {
            state.remember(Peer {
                id: hello.from.clone(),
                name: received.name,
                key: secret.clone(),
                addresses: vec![endpoint],
            })?;
        }
        finish_pairing(&mut state, &hello.from, &hello.invitation, &secret);
        let _ = tx.send(Command::SyncIncoming(received.records, received.rates));
        (state.exchange(), state.sequence)
    };
    channel.send(&exchange)?;
    let mut state = shared.lock().unwrap();
    ensure!(
        !state.stopped && state.generation == generation,
        "Sync cancelled"
    );
    state.online.insert(hello.from.clone(), Instant::now());
    state.delivered.insert(hello.from, sequence);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::random_hex;

    #[test]
    fn manual_code_discovers_a_peer_without_advertising_its_secret() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let (a_tx, a_rx) = mpsc::channel();
        let (b_tx, b_rx) = mpsc::channel();
        let a = Network::start_inner(temp.path().join("a.json"), random_hex(16)?, a_tx, false)?;
        let b = Network::start_inner(
            temp.path().join("b.json"),
            random_hex(16)?,
            b_tx.clone(),
            false,
        )?;
        let qr = Invitation::parse(&invitation(&a, &a_rx))?;
        let mut state = a.shared.lock().unwrap();
        state.invitation.as_mut().unwrap().0.pin = "0042".into();
        let offered = &state.invitation.as_ref().unwrap().0;
        let code = offered.code();
        let info = service_info(&state.config, None, Some(offered), 0)?;
        let properties = format!("{:?}", info.get_properties());
        assert!(!properties.contains(&qr.key));
        assert_eq!(
            info.get_property_val_str("pair-pin"),
            Some(qr.ticket.as_str())
        );
        assert!(info.get_property_val_str("pin").is_none());
        drop(state);
        let discovered = BTreeMap::from([(
            qr.device.clone(),
            Advertisement {
                addresses: qr.addresses.clone(),
                request: None,
                sync_request: String::new(),
                offer: info.get_property_val_str("pair-pin").unwrap().into(),
            },
        )]);
        assert!(pin::candidates(&b.shared, &BTreeMap::new()).is_err());
        let candidates = pin::candidates(&b.shared, &discovered)?;
        assert!(pin::pair(&b.shared, &b_tx, "9999", candidates.clone(), 0).is_err());
        assert_eq!(a.shared.lock().unwrap().pin_attempts, 1);
        assert!(a.shared.lock().unwrap().config.peers.is_empty());
        pin::pair(&b.shared, &b_tx, &code, candidates.clone(), 0)?;
        wait(
            &b_rx,
            |c| matches!(c, Command::SyncStatus(s) if s.peers.len() == 1),
        );
        assert!(a.shared.lock().unwrap().invitation.is_none());
        assert!(pin::pair(&b.shared, &b_tx, &code, candidates, 0).is_err());
        Ok(())
    }
    #[test]
    fn pin_limit_expiry_and_cancellation_cannot_add_a_device() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let (a_tx, a_rx) = mpsc::channel();
        let (b_tx, _) = mpsc::channel();
        let a = Network::start_inner(temp.path().join("a.json"), random_hex(16)?, a_tx, false)?;
        let b = Network::start_inner(
            temp.path().join("b.json"),
            random_hex(16)?,
            b_tx.clone(),
            false,
        )?;
        let invitation = {
            let mut state = a.shared.lock().unwrap();
            let mut invitation = Invitation::new(
                state.config.id.clone(),
                vec![SocketAddr::from((Ipv4Addr::LOCALHOST, state.config.port))],
            )?;
            invitation.pin = "0042".into();
            state.invitation = Some((
                invitation.clone(),
                Instant::now() + Duration::from_secs(300),
            ));
            invitation
        };
        let candidates = vec![(
            invitation.device.clone(),
            Advertisement {
                addresses: invitation.addresses.clone(),
                request: None,
                sync_request: String::new(),
                offer: invitation.ticket.clone(),
            },
        )];
        for attempt in 1..=5 {
            assert!(pin::pair(&b.shared, &b_tx, "9999", candidates.clone(), 0).is_err());
            let state = a.shared.lock().unwrap();
            assert_eq!(state.pin_attempts, attempt);
            if attempt < 5 {
                assert!(state.invitation.is_some());
            }
            assert!(state.config.peers.is_empty());
        }
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Command::SyncStatus(status) =
                a_rx.recv_timeout(until.saturating_duration_since(Instant::now()))?
                && status.error
                && status.invitation.is_empty()
            {
                break;
            }
        }
        assert!(pin::pair(&b.shared, &b_tx, "0042", candidates.clone(), 0).is_err());
        {
            let mut state = a.shared.lock().unwrap();
            state.pin_attempts = 0;
            state.invitation = Some((invitation, Instant::now() - Duration::from_secs(1)));
        }
        assert!(pin::pair(&b.shared, &b_tx, "0042", candidates.clone(), 0).is_err());
        a.shared.lock().unwrap().invitation = None;
        assert!(pin::pair(&b.shared, &b_tx, "0042", candidates, 0).is_err());
        assert!(a.shared.lock().unwrap().config.peers.is_empty());
        assert!(b.shared.lock().unwrap().config.peers.is_empty());
        Ok(())
    }
    #[track_caller]
    fn wait(rx: &mpsc::Receiver<Command>, predicate: impl Fn(&Command) -> bool) -> Command {
        let until = Instant::now() + Duration::from_secs(12);
        loop {
            let command = rx
                .recv_timeout(until.saturating_duration_since(Instant::now()))
                .expect("sync event before deadline");
            if let Command::SyncStatus(status) = &command {
                assert!(!status.error, "Sync failed: {}", status.message);
            }
            if predicate(&command) {
                return command;
            }
        }
    }
    fn invitation(network: &Network, rx: &mpsc::Receiver<Command>) -> String {
        network.request(Request::Invite);
        let Command::SyncStatus(status) = wait(
            rx,
            |c| matches!(c, Command::SyncStatus(s) if !s.invitation.is_empty()),
        ) else {
            unreachable!()
        };
        let mut value = Invitation::parse(&status.invitation).unwrap();
        value.addresses = vec![SocketAddr::from((
            Ipv4Addr::LOCALHOST,
            network.shared.lock().unwrap().config.port,
        ))];
        value.encode().unwrap()
    }
    #[test]
    fn qr_pairing_exchanges_encrypted_progress_reconnects_and_revokes_access() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let (a_tx, a_rx) = mpsc::channel();
        let (b_tx, b_rx) = mpsc::channel();
        let a_id = random_hex(16)?;
        let b_id = random_hex(16)?;
        let a = Network::start_inner(
            temp.path().join("a.json"),
            a_id.clone(),
            a_tx.clone(),
            false,
        )?;
        let b = Network::start_inner(temp.path().join("b.json"), b_id.clone(), b_tx, false)?;
        let record = Record {
            book: "aa".repeat(32),
            part: "bb".repeat(32),
            occurrence: 0,
            position: 41_000,
            completed: false,
            reset: false,
            updated: crate::library::now_millis(),
            device: a_id.clone(),
        };
        a.publish(vec![record.clone()], vec![]);
        let qr = invitation(&a, &a_rx);
        b.request(Request::Pair(qr));
        let Command::SyncIncoming(records, _) = wait(
            &b_rx,
            |c| matches!(c, Command::SyncIncoming(records, _) if records == &vec![record.clone()]),
        ) else {
            unreachable!()
        };
        assert_eq!(records, vec![record.clone()]);
        wait(
            &a_rx,
            |c| matches!(c, Command::SyncStatus(s) if s.peers.len() == 1 && s.invitation.is_empty()),
        );
        assert_eq!(
            Config::load(&temp.path().join("b.json"), b_id)?.peers.len(),
            1
        );
        drop(a);
        let a = Network::start_inner(temp.path().join("a.json"), a_id, a_tx, false)?;
        let mut newer = record;
        newer.updated += 1;
        newer.position = 12_000;
        a.publish(vec![newer.clone()], vec![]);
        b.request(Request::Now);
        wait(
            &b_rx,
            |c| matches!(c, Command::SyncIncoming(r, _) if r == &vec![newer.clone()]),
        );
        let peer = a.shared.lock().unwrap().config.peers[0].id.clone();
        a.request(Request::Forget(peer));
        wait(
            &a_rx,
            |c| matches!(c, Command::SyncStatus(s) if s.peers.is_empty()),
        );
        let saved_peer = b.shared.lock().unwrap().config.peers[0].clone();
        let generation = b.shared.lock().unwrap().generation;
        assert!(connect(&b.shared, &mpsc::channel().0, saved_peer, "", generation).is_err());
        Ok(())
    }
    #[test]
    fn qr_owner_can_connect_back_without_exposing_the_pairing_secret() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let (a_tx, a_rx) = mpsc::channel();
        let (b_tx, b_rx) = mpsc::channel();
        let a = Network::start_inner(
            temp.path().join("a.json"),
            random_hex(16)?,
            a_tx.clone(),
            false,
        )?;
        let b = Network::start_inner(temp.path().join("b.json"), random_hex(16)?, b_tx, false)?;
        let qr = Invitation::parse(&invitation(&a, &a_rx))?;
        let mut state = b.shared.lock().unwrap();
        state.pairing = Some((qr.clone(), Instant::now() + Duration::from_secs(300)));
        let advertised = service_info(&state.config, Some(&qr), None, 0)?;
        assert!(!format!("{:?}", advertised.get_properties()).contains(&qr.key));
        let ad = Advertisement {
            addresses: vec![SocketAddr::from((Ipv4Addr::LOCALHOST, state.config.port))],
            request: Some((qr.device.clone(), qr.ticket.clone())),
            sync_request: String::new(),
            offer: String::new(),
        };
        let id = state.config.id.clone();
        drop(state);
        let (peer, ticket) = reverse_peer(&a.shared.lock().unwrap(), &id, &ad).unwrap();
        connect(&a.shared, &a_tx, peer, &ticket, 0)?;
        wait(
            &b_rx,
            |c| matches!(c, Command::SyncStatus(s) if s.peers.len() == 1),
        );
        assert!(a.shared.lock().unwrap().invitation.is_none());
        assert!(b.shared.lock().unwrap().pairing.is_none());
        assert!(reverse_peer(&a.shared.lock().unwrap(), &id, &ad).is_none());
        Ok(())
    }
    #[test]
    fn expired_cancelled_and_wrong_qr_keys_cannot_pair() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let (a_tx, a_rx) = mpsc::channel();
        let (b_tx, _) = mpsc::channel();
        let a = Network::start_inner(temp.path().join("a.json"), random_hex(16)?, a_tx, false)?;
        let b = Network::start_inner(
            temp.path().join("b.json"),
            random_hex(16)?,
            b_tx.clone(),
            false,
        )?;
        let qr = Invitation::parse(&invitation(&a, &a_rx))?;
        let mut peer = Peer {
            id: qr.device,
            name: String::new(),
            key: random_hex(32)?,
            addresses: qr.addresses,
        };
        assert!(connect(&b.shared, &b_tx, peer.clone(), &qr.ticket, 0).is_err());
        peer.key = qr.key;
        a.shared.lock().unwrap().invitation.as_mut().unwrap().1 =
            Instant::now() - Duration::from_secs(1);
        assert!(connect(&b.shared, &b_tx, peer, &qr.ticket, 0).is_err());
        assert!(a.shared.lock().unwrap().config.peers.is_empty());
        assert!(Invitation::parse("https://example.com").is_err());
        Ok(())
    }

    #[test]
    fn one_sided_manual_sync_reports_busy_success_and_offline_failure() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let (a_tx, a_rx) = mpsc::channel();
        let (b_tx, b_rx) = mpsc::channel();
        let a = Network::start_inner(temp.path().join("a.json"), random_hex(16)?, a_tx, false)?;
        let b = Network::start_inner(temp.path().join("b.json"), random_hex(16)?, b_tx, false)?;
        b.request(Request::Pair(invitation(&a, &a_rx)));
        wait(
            &b_rx,
            |c| matches!(c, Command::SyncStatus(s) if s.peers.len() == 1),
        );
        let record = super::super::compact::tests::fixture().records.remove(0);
        {
            let mut state = a.shared.lock().unwrap();
            state.records = vec![record.clone()];
            state.config.peers[0].addresses.clear();
            state.dirty = false;
        }
        b.request(Request::Now);
        wait(&b_rx, |c| matches!(c, Command::SyncStatus(s) if s.busy));
        wait(
            &b_rx,
            |c| matches!(c, Command::SyncIncoming(r, _) if r == &vec![record.clone()]),
        );
        wait(
            &b_rx,
            |c| matches!(c, Command::SyncStatus(s) if !s.busy && !s.message.is_empty()),
        );
        drop(a);
        b.request(Request::Now);
        wait(&b_rx, |c| matches!(c, Command::SyncStatus(s) if s.busy));
        {
            let mut state = b.shared.lock().unwrap();
            state.manual = Some(Instant::now() - Duration::from_secs(9));
            state.online.clear();
        }
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Command::SyncStatus(s) =
                b_rx.recv_timeout(until.saturating_duration_since(Instant::now()))?
                && !s.busy
                && s.error
            {
                assert!(!s.message.is_empty());
                break;
            }
        }
        Ok(())
    }
}
