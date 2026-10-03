//! Private-bus integration fixture; never compiled into a release build.
use std::sync::Mutex;

#[derive(Default)]
struct Watcher(Mutex<Vec<String>>);
#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    fn register_status_notifier_item(&self, service: String) {
        self.0
            .lock()
            .unwrap()
            .push(format!("{service}/StatusNotifierItem"));
    }
    fn register_status_notifier_host(&self, _service: String) {}
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }
}
pub fn run() -> anyhow::Result<()> {
    let _connection = zbus::blocking::connection::Builder::session()?
        .name("org.kde.StatusNotifierWatcher")?
        .serve_at("/StatusNotifierWatcher", Watcher::default())?
        .build()?;
    println!("TEST TRAY HOST READY");
    loop {
        std::thread::park();
    }
}

struct Portal;
#[zbus::interface(name = "org.freedesktop.portal.FileChooser")]
impl Portal {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    async fn open_file(
        &self,
        parent_window: String,
        title: String,
        options: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &zbus::Connection,
    ) -> zbus::fdo::Result<zbus::zvariant::OwnedObjectPath> {
        let token = options
            .get("handle_token")
            .and_then(|v| <&str>::try_from(v).ok())
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs("Missing token".into()))?;
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::Failed("No sender".into()))?
            .as_str()
            .trim_start_matches(':')
            .replace('.', "_");
        let path = zbus::zvariant::OwnedObjectPath::try_from(format!(
            "/org/freedesktop/portal/desktop/request/{sender}/{token}"
        ))
        .unwrap();
        println!("TEST PORTAL PARENT {parent_window}");
        let source_folder = if title == "Выбрать папку источника" {
            assert!(
                options.contains_key("current_folder"),
                "source picker must start from its current folder"
            );
            Some(std::env::var("CARLITOS_TEST_SOURCE_FOLDER").expect("source picker fixture"))
        } else {
            None
        };
        let cloned_path = path.clone();
        let connection = connection.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let mut result: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
                Default::default();
            let response = if let Some(folder) = source_folder {
                let uri = carlitos::library::file_uri(std::path::Path::new(&folder)).unwrap();
                result.insert(
                    "uris".into(),
                    zbus::zvariant::Value::from(vec![uri]).try_into().unwrap(),
                );
                0u32
            } else {
                1u32
            };
            let _ = futures_lite::future::block_on(connection.emit_signal(
                None::<&str>,
                cloned_path,
                "org.freedesktop.portal.Request",
                "Response",
                &(response, result),
            ));
        });
        Ok(path)
    }
}
pub fn portal() -> anyhow::Result<()> {
    let _connection = zbus::blocking::connection::Builder::session()?
        .name("org.freedesktop.portal.Desktop")?
        .serve_at("/org/freedesktop/portal/desktop", Portal)?
        .serve_at(
            "/org/freedesktop/portal/desktop",
            Preferences(std::sync::atomic::AtomicU32::new(1)),
        )?
        .build()?;
    println!("TEST PORTAL READY");
    loop {
        std::thread::park();
    }
}

struct Preferences(std::sync::atomic::AtomicU32);
#[zbus::interface(name = "org.freedesktop.portal.Settings")]
impl Preferences {
    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }
    fn read(&self, _namespace: &str, _key: &str) -> zbus::zvariant::OwnedValue {
        self.0.load(std::sync::atomic::Ordering::SeqCst).into()
    }
    async fn set_scheme(
        &self,
        value: u32,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        self.0.store(value, std::sync::atomic::Ordering::SeqCst);
        Self::setting_changed(
            &emitter,
            "org.freedesktop.appearance",
            "color-scheme",
            value.into(),
        )
        .await
        .map_err(Into::into)
    }
    #[zbus(signal)]
    async fn setting_changed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        namespace: &str,
        key: &str,
        value: zbus::zvariant::OwnedValue,
    ) -> zbus::Result<()>;
}
