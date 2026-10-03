use super::mpris::PATH;
use std::{
    hash::{Hash, Hasher},
    time::Duration,
};
use zbus::blocking::Connection;

pub struct Instance {
    pub connection: Option<Connection>,
    pub activation_name: String,
    _lock: std::fs::File,
}
impl Instance {
    pub fn acquire(data: &std::path::Path, desktop: bool) -> anyhow::Result<Option<Self>> {
        std::fs::create_dir_all(data)?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        data.canonicalize()?.hash(&mut hash);
        let activation_name = format!("io.github.mny315.Carlitos.library_{:016x}", hash.finish());
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data.join("instance.lock"))?;
        if let Err(error) = lock.try_lock() {
            if let std::fs::TryLockError::Error(error) = error {
                return Err(error.into());
            }
            if desktop
                && let Ok(c) = zbus::blocking::connection::Builder::session()
                    .and_then(|b| b.method_timeout(Duration::from_secs(2)).build())
                && let Ok(p) = zbus::blocking::Proxy::new(
                    &c,
                    activation_name.as_str(),
                    PATH,
                    "org.mpris.MediaPlayer2",
                )
            {
                let _: zbus::Result<()> = p.call("Raise", &());
            }
            return Ok(None);
        }
        let connection = if desktop {
            zbus::blocking::connection::Builder::session()
                .and_then(|b| b.method_timeout(Duration::from_secs(2)).build())
                .ok()
        } else {
            None
        };
        Ok(Some(Self {
            connection,
            activation_name,
            _lock: lock,
        }))
    }
}
