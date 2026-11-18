pub(crate) fn open(destination: &str) -> Result<(), &'static str> {
    launch(destination, |target| open::that_detached(target))
}

fn launch(
    destination: &str,
    opener: impl FnOnce(&str) -> std::io::Result<()>,
) -> Result<(), &'static str> {
    if !allowed(destination) {
        return Err(
            "El destino requiere una URL http, https o mailto; las rutas relativas no están \
             resueltas.",
        );
    }
    opener(destination)
        .map_err(|_| "No se pudo iniciar la aplicación predeterminada para este enlace.")
}

pub(crate) fn fragment(source: &str) -> Option<String> {
    let mut output = Vec::with_capacity(source.len());
    let mut bytes = source.bytes();
    while let Some(byte) = bytes.next() {
        output.push(if byte == b'%' {
            let high = char::from(bytes.next()?).to_digit(16)?;
            let low = char::from(bytes.next()?).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()?
        } else {
            byte
        });
    }
    String::from_utf8(output).ok()
}

fn allowed(destination: &str) -> bool {
    if destination
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return false;
    }
    let Some((scheme, address)) = destination.split_once(':') else {
        return false;
    };
    if scheme.eq_ignore_ascii_case("mailto") {
        return address.contains('@');
    }
    (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        && address.strip_prefix("//").is_some_and(|rest| {
            !rest
                .split(['/', '?', '#'])
                .next()
                .unwrap_or_default()
                .is_empty()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_web_and_email_targets_can_launch_an_application() {
        for target in [
            "https://example.org/a?x=1&y=2",
            "http://example.org",
            "mailto:nota@example.org",
        ] {
            assert!(allowed(target));
        }
        for target in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "assets/image.png",
            "https://",
            "https://?query",
            "https://example.org/\u{1b}[2J",
            "mailto:\r\ncommand",
        ] {
            assert!(open(target).is_err(), "{target:?}");
        }
    }

    #[test]
    fn the_launcher_receives_one_validated_destination_and_failures_stay_recoverable() {
        let mut opened = None;
        launch("https://example.org/?x=1&y=2", |url| {
            opened = Some(url.to_owned());
            Ok(())
        })
        .unwrap();
        assert_eq!(opened.as_deref(), Some("https://example.org/?x=1&y=2"));
        assert!(
            launch("javascript:alert(1)", |_| panic!(
                "el destino bloqueado no debe lanzarse"
            ))
            .is_err()
        );
        assert!(
            launch("https://example.org", |_| Err(std::io::Error::other(
                "sin aplicación"
            )))
            .is_err()
        );
    }
}
