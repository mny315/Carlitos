use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    {
        crate::platform::data_dir()
    }
    #[cfg(target_os = "android")]
    {
        crate::android::context().files.clone()
    }
    #[cfg(target_os = "linux")]
    {
        xdg("XDG_DATA_HOME", ".local/share").join("carlitos")
    }
}
pub fn cache_dir() -> PathBuf {
    #[cfg(windows)]
    {
        crate::platform::data_dir().join("cache")
    }
    #[cfg(target_os = "android")]
    {
        crate::android::context().cache.clone()
    }
    #[cfg(target_os = "linux")]
    {
        xdg("XDG_CACHE_HOME", ".cache").join("carlitos")
    }
}
#[cfg(target_os = "linux")]
fn xdg(key: &str, fallback: &str) -> PathBuf {
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback)
        })
}
pub fn local_path(uri: &str) -> Option<PathBuf> {
    url::Url::parse(uri).ok()?.to_file_path().ok()
}
pub fn file_uri(path: &std::path::Path) -> anyhow::Result<String> {
    url::Url::from_file_path(path)
        .map(|u| u.to_string())
        .map_err(|_| anyhow::anyhow!(tformat!("Не удалось получить URI: {}", path.display())))
}
