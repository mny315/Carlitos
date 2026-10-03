pub(crate) fn validate_arguments(args: &[String]) -> anyhow::Result<()> {
    let mut i = 0;
    while i < args.len() {
        let key = args[i].as_str();
        let flag = matches!(key, "--help" | "--demo" | "--no-desktop" | "--fake-audio")
            || cfg!(all(debug_assertions, target_os = "linux"))
                && matches!(
                    key,
                    "--self-test"
                        | "--mock-tray-host"
                        | "--mock-portal"
                        | "--test-theme"
                        | "--test-cover-theme"
                );
        if flag {
            i += 1;
            continue;
        }
        anyhow::ensure!(
            matches!(
                key,
                "--data-dir"
                    | "--snapshot"
                    | "--quit-after"
                    | "--theme"
                    | "--language"
                    | "--page"
                    | "--size"
                    | "--open-book"
            ) || cfg!(all(debug_assertions, target_os = "linux"))
                && matches!(
                    key,
                    "--exercise" | "--test-close-after" | "--test-portal" | "--test-library-tab"
                ),
            "Unknown option: {key}. Use --help."
        );
        let value = args
            .get(i + 1)
            .filter(|s| !s.starts_with("--"))
            .ok_or_else(|| anyhow::anyhow!("Missing value for {key}"))?;
        match key {
            "--theme" => anyhow::ensure!(
                ["dark", "light", "system"].contains(&value.as_str()),
                "Invalid theme"
            ),
            "--language" => anyhow::ensure!(
                ["ru", "en", "system"].contains(&value.as_str()),
                "Invalid language"
            ),
            "--page" => anyhow::ensure!((0..=5).contains(&value.parse::<i32>()?), "Invalid page"),
            "--test-library-tab" => anyhow::ensure!(
                (0..=1).contains(&value.parse::<i32>()?),
                "Invalid library tab"
            ),
            "--quit-after" => {
                // Slint converts seconds to millisecond timestamps. A u32
                // delay leaves room for the clock on both supported platforms.
                value.parse::<u32>()?;
            }
            "--open-book" => {
                carlitos::library::Id::try_from(value.parse::<u64>()?)?;
            }
            "--size" => {
                let (w, h) = value
                    .split_once('x')
                    .ok_or_else(|| anyhow::anyhow!("Size must be WIDTHxHEIGHT"))?;
                anyhow::ensure!(
                    (360..=7680).contains(&w.parse::<u32>()?)
                        && (480..=4320).contains(&h.parse::<u32>()?),
                    "Size outside supported range"
                );
            }
            "--test-close-after" => {
                for delay in value.split(',') {
                    delay.parse::<u32>()?;
                }
            }
            _ => {}
        }
        i += 2;
    }
    if args.iter().any(|a| {
        a == "--exercise"
            || a == "--test-close-after"
            || a == "--test-portal"
            || a == "--test-theme"
            || a == "--test-library-tab"
    }) {
        anyhow::ensure!(
            args.iter().any(|a| a == "--data-dir"),
            "Test scenarios require an isolated --data-dir"
        );
    }
    Ok(())
}

#[cfg(test)]
mod argument_tests {
    use super::validate_arguments;

    #[test]
    fn timer_and_book_values_must_fit_their_consumers() {
        let validate =
            |option: &str, value: &str| validate_arguments(&[option.into(), value.into()]);
        for seconds in ["0", "3", "4294967295"] {
            assert!(validate("--quit-after", seconds).is_ok());
        }
        for seconds in ["4294967296", "18446744073709552", "18446744073709551615"] {
            assert!(validate("--quit-after", seconds).is_err());
        }
        assert!(validate("--open-book", "9223372036854775807").is_ok());
        assert!(validate("--open-book", "9223372036854775808").is_err());
        #[cfg(all(debug_assertions, target_os = "linux"))]
        for (delays, valid) in [("0,1,3", true), ("1,18446744073709552", false)] {
            assert_eq!(
                validate_arguments(&[
                    "--test-close-after".into(),
                    delays.into(),
                    "--data-dir".into(),
                    "unused-test-directory".into(),
                ])
                .is_ok(),
                valid,
            );
        }
    }
}
