//! The desktop's Secret Service (GNOME Keyring, KWallet's Secret Service) over D-Bus, through
//! GLib's own D-Bus (gio), as libsecret does: reading a secret by its attributes, and keeping
//! one. The browser keeps one key there, for its saved passwords (passwords.rs), and reads the
//! keys Chromium-based browsers keep there, to import their passwords (browsercrypto.rs).
//!
//! Blocking, and a locked keyring makes the desktop ask for its password: never call these on
//! the main thread. Secrets travel over the session bus unencrypted ("plain"), which the spec
//! allows: the bus is the person's own, on this computer.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::gio::{self, BusType, DBusCallFlags, DBusConnection, DBusSignalFlags};
use gtk::glib::variant::ObjectPath;
use gtk::glib::{self, ControlFlow, ToVariant, Variant, VariantTy};

const SERVICE: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_IFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_IFACE: &str = "org.freedesktop.Secret.Collection";
const PROMPT_IFACE: &str = "org.freedesktop.Secret.Prompt";
const SESSION_IFACE: &str = "org.freedesktop.Secret.Session";
const PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";
/// How long a prompt (unlocking the keyring, a new keyring's password) may wait for the person.
const PROMPT_WAIT: Duration = Duration::from_secs(300);
/// How long a call may take, starting the keyring service included.
const CALL_TIMEOUT_MS: i32 = 25_000;

#[derive(Debug)]
pub enum Error {
    /// No Secret Service on the session bus: no keyring on this desktop.
    Unavailable(String),
    /// The person closed the keyring's prompt, or didn't answer it.
    Dismissed,
    Failed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unavailable(why) => write!(f, "no Secret Service: {why}"),
            Error::Dismissed => write!(f, "the keyring prompt was dismissed"),
            Error::Failed(why) => write!(f, "Secret Service: {why}"),
        }
    }
}

fn failed(error: glib::Error) -> Error {
    Error::Failed(error.message().to_string())
}

fn object_path(text: &str) -> Result<ObjectPath, Error> {
    ObjectPath::try_from(text.to_string()).map_err(|e| Error::Failed(e.to_string()))
}

/// The object paths in an `ao`.
fn paths(value: &Variant) -> Vec<String> {
    value
        .iter()
        .filter_map(|path| path.str().map(str::to_string))
        .collect()
}

/// A session opened on the bus: the service ties secrets to the connection that opened it.
struct Session {
    connection: DBusConnection,
    path: String,
}

impl Session {
    fn open() -> Result<Session, Error> {
        let unavailable = |e: glib::Error| Error::Unavailable(e.message().to_string());
        let connection =
            gio::bus_get_sync(BusType::Session, None::<&gio::Cancellable>).map_err(unavailable)?;
        let reply = connection
            .call_sync(
                Some(SERVICE),
                SERVICE_PATH,
                SERVICE_IFACE,
                "OpenSession",
                Some(&("plain", "".to_variant()).to_variant()),
                VariantTy::new("(vo)").ok(),
                DBusCallFlags::NONE,
                CALL_TIMEOUT_MS,
                None::<&gio::Cancellable>,
            )
            .map_err(unavailable)?;
        let path = reply.child_value(1).str().unwrap_or_default().to_string();
        Ok(Session { connection, path })
    }

    fn call(
        &self,
        object: &str,
        interface: &str,
        method: &str,
        args: &Variant,
        reply: &str,
    ) -> Result<Variant, Error> {
        self.connection
            .call_sync(
                Some(SERVICE),
                object,
                interface,
                method,
                Some(args),
                VariantTy::new(reply).ok(),
                DBusCallFlags::NONE,
                CALL_TIMEOUT_MS,
                None::<&gio::Cancellable>,
            )
            .map_err(failed)
    }

    /// Shows a prompt the service asks for and waits for the person's answer, running a main
    /// loop of this thread's own for the answer to arrive in.
    fn prompt(&self, path: &str) -> Result<(), Error> {
        if path == "/" {
            return Ok(());
        }
        let context = glib::MainContext::new();
        let main_loop = glib::MainLoop::new(Some(&context), false);
        let dismissed: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
        let asked = context
            .with_thread_default(|| {
                // Listening before asking, so the answer can't come first.
                let subscription = {
                    let (dismissed, main_loop) = (dismissed.clone(), main_loop.clone());
                    self.connection.signal_subscribe(
                        None,
                        Some(PROMPT_IFACE),
                        Some("Completed"),
                        Some(path),
                        None,
                        DBusSignalFlags::NONE,
                        move |_, _, _, _, _, parameters| {
                            dismissed
                                .set(Some(parameters.child_value(0).get::<bool>() != Some(false)));
                            main_loop.quit();
                        },
                    )
                };
                let asked = self.call(path, PROMPT_IFACE, "Prompt", &("",).to_variant(), "()");
                if asked.is_ok() {
                    let timeout =
                        glib::timeout_source_new(PROMPT_WAIT, None, glib::Priority::DEFAULT, {
                            let main_loop = main_loop.clone();
                            move || {
                                main_loop.quit();
                                ControlFlow::Break
                            }
                        });
                    timeout.attach(Some(&context));
                    main_loop.run();
                    timeout.destroy();
                }
                self.connection.signal_unsubscribe(subscription);
                asked
            })
            .map_err(|e| Error::Failed(e.to_string()))?;
        asked?;
        match dismissed.get() {
            Some(false) => Ok(()),
            _ => Err(Error::Dismissed),
        }
    }

    fn unlock(&self, objects: &[String]) -> Result<(), Error> {
        let objects = objects
            .iter()
            .map(|o| object_path(o))
            .collect::<Result<Vec<_>, _>>()?;
        let reply = self.call(
            SERVICE_PATH,
            SERVICE_IFACE,
            "Unlock",
            &(objects,).to_variant(),
            "(aoo)",
        )?;
        self.prompt(reply.child_value(1).str().unwrap_or("/"))
    }

    /// The unlocked and the locked items with these attributes.
    fn search(
        &self,
        attributes: &HashMap<String, String>,
    ) -> Result<(Vec<String>, Vec<String>), Error> {
        let reply = self.call(
            SERVICE_PATH,
            SERVICE_IFACE,
            "SearchItems",
            &(attributes.clone(),).to_variant(),
            "(aoao)",
        )?;
        Ok((paths(&reply.child_value(0)), paths(&reply.child_value(1))))
    }

    fn close(&self) {
        let _ = self.connection.call_sync(
            Some(SERVICE),
            &self.path,
            SESSION_IFACE,
            "Close",
            None,
            None,
            DBusCallFlags::NONE,
            CALL_TIMEOUT_MS,
            None::<&gio::Cancellable>,
        );
    }
}

fn attribute_map(attributes: &[(&str, &str)]) -> HashMap<String, String> {
    attributes
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The secret of the first item with these attributes, unlocking the keyring if it has to;
/// None when there is no such item.
pub fn lookup(attributes: &[(&str, &str)]) -> Result<Option<Vec<u8>>, Error> {
    let session = Session::open()?;
    let result = (|| {
        let attributes = attribute_map(attributes);
        let (mut items, locked) = session.search(&attributes)?;
        if items.is_empty() && !locked.is_empty() {
            session.unlock(&locked)?;
            items = session.search(&attributes)?.0;
        }
        if items.is_empty() {
            return Ok(None);
        }
        let item_paths = items
            .iter()
            .map(|i| object_path(i))
            .collect::<Result<Vec<_>, _>>()?;
        let reply = session.call(
            SERVICE_PATH,
            SERVICE_IFACE,
            "GetSecrets",
            &(item_paths, object_path(&session.path)?).to_variant(),
            "(a{o(oayays)})",
        )?;
        // Each entry: the item, and its secret (session, parameters, value, content type).
        let secrets: HashMap<String, Vec<u8>> = reply
            .child_value(0)
            .iter()
            .filter_map(|entry| {
                let item = entry.child_value(0).str()?.to_string();
                let value = entry.child_value(1).child_value(2);
                Some((item, value.fixed_array::<u8>().ok()?.to_vec()))
            })
            .collect();
        Ok(items.iter().find_map(|item| secrets.get(item).cloned()))
    })();
    session.close();
    result
}

/// Keeps a secret in the default keyring, replacing an item with the same attributes. A desktop
/// with no keyring yet makes one, asking the person for its password, as libsecret does.
pub fn store(label: &str, attributes: &[(&str, &str)], secret: &[u8]) -> Result<(), Error> {
    let session = Session::open()?;
    let result = (|| {
        let alias = |session: &Session| -> Result<String, Error> {
            let reply = session.call(
                SERVICE_PATH,
                SERVICE_IFACE,
                "ReadAlias",
                &("default",).to_variant(),
                "(o)",
            )?;
            Ok(reply.child_value(0).str().unwrap_or("/").to_string())
        };
        let mut collection = alias(&session)?;
        if collection == "/" {
            let properties: HashMap<String, Variant> = HashMap::from([(
                "org.freedesktop.Secret.Collection.Label".to_string(),
                "Login".to_variant(),
            )]);
            let reply = session.call(
                SERVICE_PATH,
                SERVICE_IFACE,
                "CreateCollection",
                &(properties, "default").to_variant(),
                "(oo)",
            )?;
            session.prompt(reply.child_value(1).str().unwrap_or("/"))?;
            collection = match reply.child_value(0).str() {
                Some(made) if made != "/" => made.to_string(),
                _ => alias(&session)?,
            };
            if collection == "/" {
                return Err(Error::Failed("no keyring to keep the secret in".into()));
            }
        }
        let locked = session
            .call(
                &collection,
                PROPERTIES_IFACE,
                "Get",
                &(COLLECTION_IFACE, "Locked").to_variant(),
                "(v)",
            )?
            .child_value(0)
            .as_variant()
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false);
        if locked {
            session.unlock(std::slice::from_ref(&collection))?;
        }
        let properties: HashMap<String, Variant> = HashMap::from([
            (
                "org.freedesktop.Secret.Item.Label".to_string(),
                label.to_variant(),
            ),
            (
                "org.freedesktop.Secret.Item.Attributes".to_string(),
                attribute_map(attributes).to_variant(),
            ),
        ]);
        let secret = (
            object_path(&session.path)?,
            Vec::<u8>::new(),
            secret.to_vec(),
            "text/plain",
        );
        let reply = session.call(
            &collection,
            COLLECTION_IFACE,
            "CreateItem",
            &(properties, secret, true).to_variant(),
            "(oo)",
        )?;
        session.prompt(reply.child_value(1).str().unwrap_or("/"))
    })();
    session.close();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Only against a keyring of its own: a D-Bus session and a gnome-keyring started for the
    /// test, never the person's (PEEPWICK_TEST_KEYRING=1 says it is one).
    #[test]
    #[ignore]
    fn keeps_and_finds_a_secret_in_a_test_keyring() {
        assert_eq!(std::env::var("PEEPWICK_TEST_KEYRING").as_deref(), Ok("1"));
        let run = format!("{:?}", std::time::SystemTime::now());
        let attributes = [("application", "peepwick-test"), ("run", run.as_str())];
        assert_eq!(lookup(&attributes).unwrap(), None);
        store("PEEPWICK test", &attributes, b"first").unwrap();
        assert_eq!(lookup(&attributes).unwrap().as_deref(), Some(&b"first"[..]));
        // The same attributes: the item is replaced, not added.
        store("PEEPWICK test", &attributes, "ikinci şifre".as_bytes()).unwrap();
        assert_eq!(
            lookup(&attributes).unwrap().as_deref(),
            Some("ikinci şifre".as_bytes())
        );
    }

    /// A session bus with no Secret Service on it (PEEPWICK_TEST_KEYRING=none).
    #[test]
    #[ignore]
    fn says_when_there_is_no_keyring() {
        assert_eq!(
            std::env::var("PEEPWICK_TEST_KEYRING").as_deref(),
            Ok("none")
        );
        let started = std::time::Instant::now();
        assert!(matches!(
            lookup(&[("application", "peepwick-test")]),
            Err(Error::Unavailable(_))
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
