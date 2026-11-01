//! Site security: what the lock button shows (the connection, the certificate, the site's
//! permissions and cookies), the certificate viewer, certificate warnings people chose to
//! skip, and HTTPS-Only mode, which tries every plain-HTTP page over HTTPS first.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gtk::gio;
use gtk::glib;
use gtk::glib::translate::{IntoGlib, ToGlibPtr};
use serde_json::{Value, json};
use webkit2gtk::{
    CookieManagerExt, WebContextExt, WebViewExt, WebsiteData, WebsiteDataManager,
    WebsiteDataManagerExt, WebsiteDataTypes,
};
use x509_parser::prelude::*;

use crate::tab::Tab;
use crate::window::{self, BrowserWindow};
use crate::{pages, permissions, settings, shield};

/// How long an upgraded page gets to start answering before HTTP is offered.
const ANSWER: Duration = Duration::from_secs(8);

struct Attempt {
    /// HTTP addresses upgraded since the last page was shown, to spot redirect loops.
    upgraded: Vec<(String, String)>,
    timer: Option<glib::SourceId>,
}

thread_local! {
    /// Hosts whose certificate warning was skipped this session.
    static EXCEPTIONS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// Sites opened over HTTP on purpose this session, with their subdomains.
    static HTTP_ALLOWED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static ATTEMPTS: RefCell<HashMap<u32, Attempt>> = RefCell::new(HashMap::new());
}

// ── Certificates ──────────────────────────────────────────────────────────────

/// A certificate problem, as the UI knows them: Chromium's error name and code.
pub fn verification(flags: gio::TlsCertificateFlags) -> (&'static str, i32) {
    use gio::TlsCertificateFlags as F;
    if flags.is_empty() {
        ("net::OK", 0)
    } else if flags.contains(F::UNKNOWN_CA) {
        ("net::ERR_CERT_AUTHORITY_INVALID", -202)
    } else if flags.contains(F::BAD_IDENTITY) {
        ("net::ERR_CERT_COMMON_NAME_INVALID", -200)
    } else if flags.intersects(F::NOT_ACTIVATED | F::EXPIRED) {
        ("net::ERR_CERT_DATE_INVALID", -201)
    } else if flags.contains(F::REVOKED) {
        ("net::ERR_CERT_REVOKED", -206)
    } else if flags.contains(F::INSECURE) {
        ("net::ERR_CERT_WEAK_SIGNATURE_ALGORITHM", -208)
    } else {
        ("net::ERR_CERT_INVALID", -207)
    }
}

/// AB:CD:… of a checksum.
fn fingerprint(kind: glib::ChecksumType, der: &[u8]) -> String {
    let mut checksum = glib::Checksum::new(kind).expect("SHA checksums exist");
    checksum.update(der);
    let hex = checksum
        .string()
        .map(|s| s.to_uppercase())
        .unwrap_or_default();
    hex.as_bytes()
        .chunks(2)
        .map(|pair| String::from_utf8_lossy(pair).into_owned())
        .collect::<Vec<_>>()
        .join(":")
}

fn first(values: impl Iterator<Item = String>) -> String {
    values.into_iter().next().unwrap_or_default()
}

/// An attribute's text; names some encoders store as Teletex or IA5 strings are read as bytes.
fn text_of(attribute: &AttributeTypeAndValue) -> String {
    match attribute.as_str() {
        Ok(text) => text.to_string(),
        Err(_) => String::from_utf8_lossy(attribute.attr_value().data.as_ref()).into_owned(),
    }
}

fn common_name(name: &X509Name) -> String {
    let cn = first(name.iter_common_name().map(text_of));
    if cn.is_empty() {
        first(name.iter_organization().map(text_of))
    } else {
        cn
    }
}

fn organization(name: &X509Name) -> String {
    first(name.iter_organization().map(text_of))
}

fn der_of(certificate: &gio::TlsCertificate) -> Option<Vec<u8>> {
    use gio::prelude::TlsCertificateExt;
    certificate.certificate().map(|der| der.to_vec())
}

/// What the certificate viewer shows: who it is for and from, when, the names it covers,
/// fingerprints and the chain above it.
pub fn certificate_info(certificate: &gio::TlsCertificate) -> Option<Value> {
    use gio::prelude::TlsCertificateExt;
    let der = der_of(certificate)?;
    let (_, cert) = parse_x509_certificate(&der).ok()?;
    let names: Vec<String> = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|san| {
            san.value
                .general_names
                .iter()
                .filter_map(|name| match name {
                    GeneralName::DNSName(dns) => Some(dns.to_string()),
                    GeneralName::IPAddress(bytes) if bytes.len() == 4 => Some(
                        bytes
                            .iter()
                            .map(u8::to_string)
                            .collect::<Vec<_>>()
                            .join("."),
                    ),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let key_type = match cert.public_key().parsed() {
        Ok(x509_parser::public_key::PublicKey::RSA(rsa)) => format!("RSA {}", rsa.key_size()),
        Ok(x509_parser::public_key::PublicKey::EC(ec)) => format!("EC {}", ec.key_size()),
        Ok(other) => format!("{other:?}")
            .split('(')
            .next()
            .unwrap_or("")
            .to_string(),
        Err(_) => String::new(),
    };
    let signature = x509_parser::objects::oid2sn(
        &cert.signature_algorithm.algorithm,
        x509_parser::objects::oid_registry(),
    )
    .map(str::to_string)
    .unwrap_or_else(|_| cert.signature_algorithm.algorithm.to_id_string());
    let mut chain = Vec::new();
    let mut issuer = certificate.issuer();
    while let Some(next) = issuer {
        if chain.len() >= 8 {
            break;
        }
        if let Some(der) = der_of(&next)
            && let Ok((_, above)) = parse_x509_certificate(&der)
        {
            chain.push(json!({
                "subject": common_name(above.subject()),
                "issuer": common_name(above.issuer()),
                "validTo": above.validity().not_after.timestamp() * 1000,
            }));
        }
        issuer = next.issuer();
    }
    Some(json!({
        "subject": common_name(cert.subject()),
        "subjectOrganization": organization(cert.subject()),
        "issuer": common_name(cert.issuer()),
        "issuerOrganization": organization(cert.issuer()),
        "validFrom": cert.validity().not_before.timestamp() * 1000,
        "validTo": cert.validity().not_after.timestamp() * 1000,
        "names": names,
        "serialNumber": cert.raw_serial_as_string().to_uppercase(),
        "sha256": fingerprint(glib::ChecksumType::Sha256, &der),
        "sha1": fingerprint(glib::ChecksumType::Sha1, &der),
        "keyType": key_type,
        "signatureAlgorithm": signature,
        "chain": chain,
    }))
}

pub fn has_exception(host: &str) -> bool {
    EXCEPTIONS.with(|set| set.borrow().contains(host))
}

/// The person chose to continue past a certificate warning for this host.
pub fn allow_certificate(win: &BrowserWindow, certificate: &gio::TlsCertificate, host: &str) {
    if let Some(context) = window::web_context(win.incognito) {
        context.allow_tls_certificate_for_host(certificate, host);
    }
    EXCEPTIONS.with(|set| set.borrow_mut().insert(host.to_string()));
}

/// Shows the warning again on the next load. WebKit can't forget an allowed certificate, so
/// the tab puts the warning back up when the page arrives (see `Tab`).
pub fn revoke(host: &str) {
    EXCEPTIONS.with(|set| set.borrow_mut().remove(host));
}

/// The certificate and its problems for a tab: from the page shown, or from the warning.
fn certificate_of(tab: &Tab) -> Option<(gio::TlsCertificate, gio::TlsCertificateFlags)> {
    tab.tls_failure().or_else(|| {
        tab.view()
            .filter(|_| tab.shows_web())
            .and_then(|view| view.tls_info())
    })
}

/// The certificate viewer, for a host shown in one of the window's tabs.
pub fn certificate(win: &BrowserWindow, host: &str) -> Value {
    let tabs = win.active_tab().into_iter().chain(win.all_tabs());
    for tab in tabs {
        if pages::host_of(&tab.display_url()) != host {
            continue;
        }
        if let Some((cert, flags)) = certificate_of(&tab)
            && let Some(info) = certificate_info(&cert)
        {
            let (name, code) = verification(flags);
            return json!({ "host": host, "certificate": info, "verification": name, "errorCode": code, "knownRoot": flags.is_empty() });
        }
    }
    Value::Null
}

// ── The lock button ───────────────────────────────────────────────────────────

/// The active tab's site: its connection, certificate, permissions and cookie count.
pub fn site_info(win: &BrowserWindow, done: impl FnOnce(Value) + 'static) {
    let Some(tab) = win.active_tab() else {
        return done(Value::Null);
    };
    let url = tab.display_url();
    let host = pages::host_of(&url);
    let origin = permissions::origin_of(&url);
    let (certificate, verification_name, code, known) = match certificate_of(&tab) {
        Some((cert, flags)) if url.starts_with("https:") => {
            let (name, code) = verification(flags);
            (certificate_info(&cert), Some(name), code, flags.is_empty())
        }
        _ => (None, None, 0, false),
    };
    let mut info = json!({
        "url": url,
        "host": host,
        "origin": origin,
        "security": tab.state().security,
        "verification": verification_name,
        "errorCode": code,
        "knownRoot": known,
        "exception": has_exception(&host),
        "certificate": certificate,
        "permissions": permissions::for_site(&origin, win.incognito),
        "cookies": 0,
    });
    let cookies = window::web_context(win.incognito)
        .and_then(|context| context.website_data_manager())
        .and_then(|manager| manager.cookie_manager());
    match cookies {
        Some(cookies) if url.starts_with("http") => {
            cookies.cookies(&url, None::<&gio::Cancellable>, move |result| {
                info["cookies"] = json!(result.map(|list| list.len()).unwrap_or(0));
                done(info);
            })
        }
        _ => done(info),
    }
}

/// Removes site data through WebKit's C API, which the Rust bindings leave out.
fn remove_data(
    manager: &WebsiteDataManager,
    data: &[WebsiteData],
    done: impl FnOnce(bool) + 'static,
) {
    type Done = Box<dyn FnOnce(bool)>;
    unsafe extern "C" fn finished(
        source: *mut glib::gobject_ffi::GObject,
        result: *mut gio::ffi::GAsyncResult,
        user_data: glib::ffi::gpointer,
    ) {
        unsafe {
            let done: Box<Done> = Box::from_raw(user_data as *mut Done);
            let mut error = std::ptr::null_mut();
            let ok = webkit2gtk::ffi::webkit_website_data_manager_remove_finish(
                source as *mut _,
                result,
                &mut error,
            ) != 0;
            if !error.is_null() {
                glib::ffi::g_error_free(error);
            }
            done(ok);
        }
    }
    let done: Box<Done> = Box::new(Box::new(done));
    unsafe {
        let mut list: *mut glib::ffi::GList = std::ptr::null_mut();
        for item in data {
            let pointer: *mut webkit2gtk::ffi::WebKitWebsiteData = item.to_glib_none().0;
            list = glib::ffi::g_list_prepend(list, pointer as glib::ffi::gpointer);
        }
        webkit2gtk::ffi::webkit_website_data_manager_remove(
            manager.to_glib_none().0,
            WebsiteDataTypes::all().into_glib(),
            list,
            std::ptr::null_mut(),
            Some(finished),
            Box::into_raw(done) as glib::ffi::gpointer,
        );
        glib::ffi::g_list_free(list);
    }
}

/// "Delete cookies and site data" in the lock panel: everything the active tab's site keeps.
pub fn clear_site_data(win: &BrowserWindow, done: impl FnOnce(Value) + 'static) {
    let url = win
        .active_tab()
        .map(|tab| tab.display_url())
        .unwrap_or_default();
    let Some(site) = shield::site_of(&url).filter(|_| url.starts_with("http")) else {
        return done(json!(false));
    };
    let Some(manager) =
        window::web_context(win.incognito).and_then(|context| context.website_data_manager())
    else {
        return done(json!(false));
    };
    let target = manager.clone();
    manager.fetch(
        WebsiteDataTypes::all(),
        None::<&gio::Cancellable>,
        move |result| {
            let data: Vec<WebsiteData> = result
                .unwrap_or_default()
                .into_iter()
                .filter(|d| {
                    d.name()
                        .is_some_and(|name| name == site || name.ends_with(&format!(".{site}")))
                })
                .collect();
            if data.is_empty() {
                return done(json!(true));
            }
            remove_data(&target, &data, move |ok| done(json!(ok)));
        },
    );
}

// ── HTTPS-Only ────────────────────────────────────────────────────────────────

pub enum Upgrade {
    /// Load this HTTPS address instead.
    Https(String),
    /// The secure version sent the tab back to HTTP: ask.
    Warn,
}

/// Opening a site over HTTP from the warning: its pages stay on HTTP this session.
pub fn allow_http(url: &str) {
    if let Some(site) = shield::site_of(url) {
        HTTP_ALLOWED.with(|set| set.borrow_mut().insert(site));
    }
}

fn upgradable(url: &str) -> bool {
    if !url.starts_with("http:") || !settings::get().https_only {
        return false;
    }
    let host = pages::host_of(url);
    let allowed = shield::site_of(url)
        .is_some_and(|site| HTTP_ALLOWED.with(|set| set.borrow().contains(&site)));
    !host.is_empty() && !pages::is_local_host(&host) && !allowed
}

fn to_https(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) => {
            let _ = parsed.set_scheme("https");
            // An explicit :80 belongs to HTTP; HTTPS on that port would never answer.
            if parsed.port() == Some(80) {
                let _ = parsed.set_port(None);
            }
            parsed.to_string()
        }
        Err(_) => url.replacen("http:", "https:", 1),
    }
}

fn clear(tab_id: u32) {
    if let Some(attempt) = ATTEMPTS.with(|attempts| attempts.borrow_mut().remove(&tab_id))
        && let Some(timer) = attempt.timer
    {
        timer.remove();
    }
}

/// For a top-level navigation: the HTTPS address to load instead, a warning when the secure
/// version sent the tab back to HTTP, or nothing to leave it alone.
pub fn https_upgrade(tab: &Rc<Tab>, url: &str) -> Option<Upgrade> {
    if !upgradable(url) {
        return None;
    }
    let looped = ATTEMPTS.with(|attempts| {
        attempts
            .borrow()
            .get(&tab.id)
            .is_some_and(|a| a.upgraded.iter().any(|(http, _)| http == url))
    });
    if looped {
        clear(tab.id);
        return Some(Upgrade::Warn);
    }
    let secure = to_https(url);
    let weak = Rc::downgrade(tab);
    let http = url.to_string();
    let timer = glib::timeout_add_local_once(ANSWER, move || {
        let Some(tab) = weak.upgrade() else { return };
        ATTEMPTS.with(|attempts| {
            if let Some(attempt) = attempts.borrow_mut().get_mut(&tab.id) {
                attempt.timer = None;
            }
        });
        clear(tab.id);
        tab.show_https_only(&http);
    });
    ATTEMPTS.with(|attempts| {
        let mut attempts = attempts.borrow_mut();
        let attempt = attempts.entry(tab.id).or_insert_with(|| Attempt {
            upgraded: Vec::new(),
            timer: None,
        });
        if let Some(old) = attempt.timer.replace(timer) {
            old.remove();
        }
        attempt.upgraded.push((url.to_string(), secure.clone()));
    });
    Some(Upgrade::Https(secure))
}

/// The upgraded page answered and is showing: the attempt is over.
pub fn https_settled(tab_id: u32) {
    clear(tab_id);
}

/// When a load upgraded to HTTPS fails, the HTTP address it came from. A host that can't be
/// found would fail over HTTP too, so that keeps the normal error page.
pub fn https_fallback(tab_id: u32, failed: &str, code: i32) -> Option<String> {
    if matches!(code, -105 | -106 | -137 | -21) {
        return None;
    }
    let http = ATTEMPTS.with(|attempts| {
        attempts.borrow().get(&tab_id).and_then(|a| {
            a.upgraded
                .iter()
                .find(|(_, secure)| {
                    secure == failed || pages::host_of(secure) == pages::host_of(failed)
                })
                .map(|(http, _)| http.clone())
        })
    })?;
    clear(tab_id);
    Some(http)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upgrades_addresses() {
        assert_eq!(
            to_https("http://example.com/a?b=1"),
            "https://example.com/a?b=1"
        );
        assert_eq!(to_https("http://example.com:80/"), "https://example.com/");
        assert_eq!(
            to_https("http://example.com:8080/"),
            "https://example.com:8080/"
        );
    }

    #[test]
    fn names_certificate_problems() {
        use gio::TlsCertificateFlags as F;
        assert_eq!(verification(F::empty()), ("net::OK", 0));
        assert_eq!(verification(F::UNKNOWN_CA | F::EXPIRED).1, -202);
        assert_eq!(verification(F::EXPIRED).1, -201);
    }
}
