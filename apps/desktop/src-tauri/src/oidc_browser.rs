use url::Url;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OidcBrowserLaunchFailure {
    InvalidAuthorizationUrl,
    BrowserLaunchFailed,
}

pub(crate) fn launch_oidc_authorization_url<E>(
    authorization_url: &str,
    launch: impl FnOnce(&str) -> Result<(), E>,
) -> Result<(), OidcBrowserLaunchFailure> {
    let parsed = Url::parse(authorization_url)
        .map_err(|_| OidcBrowserLaunchFailure::InvalidAuthorizationUrl)?;
    let has_userinfo = authorization_url
        .split_once("://")
        .and_then(|(_, remainder)| remainder.split(['/', '?', '#']).next())
        .is_some_and(|authority| authority.contains('@'));
    if !matches!(parsed.scheme(), "http" | "https")
        || authorization_url.contains('\\')
        || has_userinfo
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(OidcBrowserLaunchFailure::InvalidAuthorizationUrl);
    }
    launch(authorization_url).map_err(|_| OidcBrowserLaunchFailure::BrowserLaunchFailed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oidc_browser_launches_original_http_and_https_urls() {
        for authorization_url in [
            "http://127.0.0.1/authorize?opaque=one%2Btwo",
            "https://identity.example.invalid/authorize?opaque=one%2Btwo",
        ] {
            let mut launched = None;
            assert_eq!(
                launch_oidc_authorization_url(authorization_url, |url| {
                    launched = Some(url.to_owned());
                    Ok::<(), ()>(())
                }),
                Ok(())
            );
            assert_eq!(launched.as_deref(), Some(authorization_url));
        }
    }

    #[test]
    fn oidc_browser_rejects_malformed_non_http_and_userinfo_urls_without_launching() {
        for authorization_url in [
            "not a url",
            "ftp://identity.example.invalid/authorize",
            "https://@identity.example.invalid/authorize",
            "https://user@identity.example.invalid/authorize",
            "https://user:password@identity.example.invalid/authorize",
            "https:user@identity.example.invalid/authorize",
            r"https:/\/@identity.example.invalid/authorize",
        ] {
            let mut launched = false;
            assert_eq!(
                launch_oidc_authorization_url(authorization_url, |_| {
                    launched = true;
                    Ok::<(), ()>(())
                }),
                Err(OidcBrowserLaunchFailure::InvalidAuthorizationUrl)
            );
            assert!(!launched);
        }
    }

    #[test]
    fn oidc_browser_maps_native_failure_without_retaining_error_text() {
        let result = launch_oidc_authorization_url(
            "https://identity.example.invalid/authorize?opaque=private",
            |_| Err("native error with private details"),
        );
        assert_eq!(result, Err(OidcBrowserLaunchFailure::BrowserLaunchFailed));
        let debug = format!("{result:?}");
        assert!(!debug.contains("identity.example.invalid"));
        assert!(!debug.contains("opaque"));
        assert!(!debug.contains("native error"));
    }
}
