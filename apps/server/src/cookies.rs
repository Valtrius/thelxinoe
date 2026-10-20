//! Every cookie Thelxinoe sets or reads, each with one name, scope and lifetime.
use axum::http::{HeaderMap, HeaderValue, header};
use std::borrow::Cow;
use thelxinoe_auth::lifetime;

pub(crate) enum Cookie<'a> {
    /// The web session.
    Session,
    /// A browser remembered after two-step sign-in, so it skips the code.
    Remember,
    /// Ties an identity-provider sign-in to the browser that started it.
    Oidc,
    /// Ties YouTube account linking to the browser that started it.
    YoutubeLink,
    /// A service's browser grant, sent only under the service's mount path.
    Service { id: &'a str, mount: &'a str },
}

impl Cookie<'_> {
    fn name(&self) -> Cow<'_, str> {
        match self {
            Self::Session => "thelxinoe_session".into(),
            Self::Remember => "thelxinoe_remember".into(),
            Self::Oidc => "thelxinoe_oidc".into(),
            Self::YoutubeLink => "thelxinoe_youtube_oauth".into(),
            Self::Service { id, .. } => format!("thelxinoe_service_{id}").into(),
        }
    }
    /// SameSite, path and lifetime. Only a cookie that must come back through a provider's
    /// cross-site redirect is Lax, and only for that callback and a few minutes. Sign-in
    /// cookies last as long as any session can; the server decides when one ends.
    fn scope(&self) -> (&'static str, Cow<'_, str>, i64) {
        match self {
            Self::Session | Self::Remember => ("Strict", "/".into(), lifetime::MAX),
            Self::Oidc => ("Lax", "/api/v1/auth/oidc".into(), 300),
            Self::YoutubeLink => ("Lax", "/api/v1/online/youtube/callback".into(), 600),
            Self::Service { mount, .. } => ("Strict", format!("{mount}/").into(), 8 * 60 * 60),
        }
    }
}

/// Every cookie the request carries, across all its Cookie headers.
pub(crate) fn pairs(headers: &HeaderMap) -> impl Iterator<Item = (&str, &str)> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(|h| h.split(';'))
        .filter_map(|part| part.trim().split_once('='))
}

/// The cookie's value if the request carries it exactly once. A second one with the same
/// name, such as one a sibling host planted, makes it unclear which was meant, so neither is.
pub(crate) fn get(headers: &HeaderMap, cookie: &Cookie) -> Option<String> {
    let name = cookie.name();
    let mut values = pairs(headers).filter(|(key, _)| *key == name);
    let (_, value) = values.next()?;
    values.next().is_none().then(|| value.to_owned())
}

/// Sets the cookie on a response: always HttpOnly, and Secure when the request was HTTPS.
pub(crate) fn set(headers: &mut HeaderMap, cookie: &Cookie, value: &str, secure: bool) {
    let max_age = cookie.scope().2;
    append(headers, cookie, value, max_age, secure);
}

/// Tells the browser to drop the cookie.
pub(crate) fn clear(headers: &mut HeaderMap, cookie: &Cookie, secure: bool) {
    append(headers, cookie, "", 0, secure);
}

fn append(headers: &mut HeaderMap, cookie: &Cookie, value: &str, max_age: i64, secure: bool) {
    let (same_site, path, _) = cookie.scope();
    let line = format!(
        "{}={value}; HttpOnly; SameSite={same_site}; Path={path}; Max-Age={max_age}{}",
        cookie.name(),
        if secure { "; Secure" } else { "" }
    );
    headers.append(
        header::SET_COOKIE,
        HeaderValue::from_str(&line).expect("cookie values and paths are header-safe"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    // A planted second cookie with the session's name must not pick which session is used.
    #[test]
    fn a_cookie_sent_twice_counts_as_absent() {
        let headers = |values: &[&str]| {
            let mut map = HeaderMap::new();
            for value in values {
                map.append(header::COOKIE, value.parse().unwrap());
            }
            map
        };
        let once = headers(&["a=1; thelxinoe_session=mine; b=2"]);
        assert_eq!(get(&once, &Cookie::Session).as_deref(), Some("mine"));
        for twice in [
            headers(&["thelxinoe_session=mine; thelxinoe_session=planted"]),
            headers(&["thelxinoe_session=mine", "thelxinoe_session=planted"]),
        ] {
            assert_eq!(get(&twice, &Cookie::Session), None);
        }
        assert_eq!(get(&once, &Cookie::Remember), None);
    }

    #[test]
    fn each_cookie_is_set_with_its_own_scope() {
        let mut response = HeaderMap::new();
        set(&mut response, &Cookie::Session, "s", true);
        set(&mut response, &Cookie::Oidc, "o", false);
        let service = Cookie::Service {
            id: "sonarr",
            mount: "/services/sonarr",
        };
        set(&mut response, &service, "g", true);
        clear(&mut response, &Cookie::Session, true);
        let lines: Vec<_> = response
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect();
        assert_eq!(
            lines,
            [
                "thelxinoe_session=s; HttpOnly; SameSite=Strict; Path=/; Max-Age=34560000; Secure",
                "thelxinoe_oidc=o; HttpOnly; SameSite=Lax; Path=/api/v1/auth/oidc; Max-Age=300",
                "thelxinoe_service_sonarr=g; HttpOnly; SameSite=Strict; Path=/services/sonarr/; Max-Age=28800; Secure",
                "thelxinoe_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0; Secure",
            ]
        );
    }
}
