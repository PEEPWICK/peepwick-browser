//! The desktop's Secret Service (GNOME Keyring, KWallet's Secret Service) over D-Bus: reading
//! a secret by its attributes, and keeping one. The browser keeps one key there, for its saved
//! passwords (passwords.rs).
//!
//! Blocking, and a locked keyring makes the desktop ask for its password: never call these on
//! the main thread. Secrets travel over the session bus unencrypted ("plain"), which the spec
//! allows: the bus is the person's own, on this computer.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const SERVICE: &str = "org.freedesktop.secrets";
const SERVICE_PATH: &str = "/org/freedesktop/secrets";
const SERVICE_IFACE: &str = "org.freedesktop.Secret.Service";
const COLLECTION_IFACE: &str = "org.freedesktop.Secret.Collection";
const PROMPT_IFACE: &str = "org.freedesktop.Secret.Prompt";
const SESSION_IFACE: &str = "org.freedesktop.Secret.Session";
/// How long a prompt (unlocking the keyring, a new keyring's password) may wait for the person.
const PROMPT_WAIT: Duration = Duration::from_secs(300);

/// A secret as the service sends it: session, parameters, value, content type.
type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

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

fn failed(error: zbus::Error) -> Error {
    Error::Failed(error.to_string())
}

/// A connection with a session open on it: the service ties secrets to the connection that
/// opened the session.
struct Session {
    connection: Connection,
    path: OwnedObjectPath,
}

impl Session {
    fn open() -> Result<Session, Error> {
        let unavailable = |e: zbus::Error| Error::Unavailable(e.to_string());
        let connection = Connection::session().map_err(unavailable)?;
        let service =
            Proxy::new(&connection, SERVICE, SERVICE_PATH, SERVICE_IFACE).map_err(unavailable)?;
        let (_, path): (OwnedValue, OwnedObjectPath) = service
            .call("OpenSession", &("plain", Value::from("")))
            .map_err(unavailable)?;
        Ok(Session { connection, path })
    }

    fn service(&self) -> Result<Proxy<'_>, Error> {
        Proxy::new(&self.connection, SERVICE, SERVICE_PATH, SERVICE_IFACE).map_err(failed)
    }

    /// Shows a prompt the service asks for and waits for the person's answer.
    fn prompt(&self, path: &OwnedObjectPath) -> Result<(), Error> {
        if path.as_str() == "/" {
            return Ok(());
        }
        let prompt =
            Proxy::new(&self.connection, SERVICE, path.as_str(), PROMPT_IFACE).map_err(failed)?;
        // Listening before asking, so the answer can't come first.
        let mut completed = prompt.receive_signal("Completed").map_err(failed)?;
        prompt.call_method("Prompt", &("",)).map_err(failed)?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let answer = completed.next().map(|message| {
                message
                    .body()
                    .deserialize::<(bool, OwnedValue)>()
                    .map(|(dismissed, _)| dismissed)
            });
            let _ = tx.send(answer);
        });
        match rx.recv_timeout(PROMPT_WAIT) {
            Ok(Some(Ok(false))) => Ok(()),
            Ok(Some(Err(error))) => Err(failed(error)),
            _ => Err(Error::Dismissed),
        }
    }

    fn unlock(&self, objects: &[OwnedObjectPath]) -> Result<(), Error> {
        let (_, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) = self
            .service()?
            .call("Unlock", &(objects,))
            .map_err(failed)?;
        self.prompt(&prompt)
    }

    fn search(
        &self,
        attributes: &HashMap<&str, &str>,
    ) -> Result<(Vec<OwnedObjectPath>, Vec<OwnedObjectPath>), Error> {
        self.service()?
            .call("SearchItems", &(attributes,))
            .map_err(failed)
    }

    fn close(&self) {
        if let Ok(session) =
            Proxy::new(&self.connection, SERVICE, self.path.as_str(), SESSION_IFACE)
        {
            let _ = session.call_method("Close", &());
        }
    }
}

/// The secret of the first item with these attributes, unlocking the keyring if it has to;
/// None when there is no such item.
pub fn lookup(attributes: &[(&str, &str)]) -> Result<Option<Vec<u8>>, Error> {
    let session = Session::open()?;
    let result = (|| {
        let attributes: HashMap<&str, &str> = attributes.iter().copied().collect();
        let (mut items, locked) = session.search(&attributes)?;
        if items.is_empty() && !locked.is_empty() {
            session.unlock(&locked)?;
            items = session.search(&attributes)?.0;
        }
        if items.is_empty() {
            return Ok(None);
        }
        let secrets: HashMap<OwnedObjectPath, Secret> = session
            .service()?
            .call("GetSecrets", &(&items, &session.path))
            .map_err(failed)?;
        Ok(items
            .iter()
            .find_map(|item| secrets.get(item))
            .map(|(_, _, value, _)| value.clone()))
    })();
    session.close();
    result
}

/// Keeps a secret in the default keyring, replacing an item with the same attributes. A desktop
/// with no keyring yet makes one, asking the person for its password, as libsecret does.
pub fn store(label: &str, attributes: &[(&str, &str)], secret: &[u8]) -> Result<(), Error> {
    let session = Session::open()?;
    let result = (|| {
        let service = session.service()?;
        let mut collection: OwnedObjectPath =
            service.call("ReadAlias", &("default",)).map_err(failed)?;
        if collection.as_str() == "/" {
            let properties = HashMap::from([(
                "org.freedesktop.Secret.Collection.Label",
                Value::from("Login"),
            )]);
            let (made, prompt): (OwnedObjectPath, OwnedObjectPath) = service
                .call("CreateCollection", &(properties, "default"))
                .map_err(failed)?;
            session.prompt(&prompt)?;
            collection = if made.as_str() == "/" {
                service.call("ReadAlias", &("default",)).map_err(failed)?
            } else {
                made
            };
            if collection.as_str() == "/" {
                return Err(Error::Failed("no keyring to keep the secret in".into()));
            }
        }
        let keyring = Proxy::new(
            &session.connection,
            SERVICE,
            collection.as_str(),
            COLLECTION_IFACE,
        )
        .map_err(failed)?;
        if keyring.get_property::<bool>("Locked").map_err(failed)? {
            session.unlock(std::slice::from_ref(&collection))?;
        }
        let attributes: HashMap<&str, &str> = attributes.iter().copied().collect();
        let properties = HashMap::from([
            ("org.freedesktop.Secret.Item.Label", Value::from(label)),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Value::from(attributes),
            ),
        ]);
        let secret = (
            &session.path,
            Vec::<u8>::new(),
            secret.to_vec(),
            "text/plain",
        );
        let (_, prompt): (OwnedObjectPath, OwnedObjectPath) = keyring
            .call("CreateItem", &(properties, secret, true))
            .map_err(failed)?;
        session.prompt(&prompt)
    })();
    session.close();
    result
}
