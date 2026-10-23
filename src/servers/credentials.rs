use uuid::Uuid;
use zeroize::{Zeroize, ZeroizeOnDrop};

#[derive(Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct Secret(pub String);

fn entry(id: Uuid) -> Result<keyring::Entry, String> {
    keyring::Entry::new("ContainerSafe.SSH", &id.to_string())
        .map_err(|_| "OS credential store unavailable".into())
}
pub fn read(id: Uuid) -> Result<Secret, String> {
    read_optional(id)?.ok_or_else(|| "Enter the password/passphrase for this server".into())
}
pub fn read_optional(id: Uuid) -> Result<Option<Secret>, String> {
    match entry(id)?.get_password() {
        Ok(value) => Ok(Some(Secret(value))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(
            "Cannot read OS credential store; unlock it before modifying saved credentials".into(),
        ),
    }
}
pub fn write(id: Uuid, secret: &Secret) -> Result<(), String> {
    entry(id)?
        .set_password(&secret.0)
        .map_err(|_| "Cannot save credential in OS credential store".into())
}
pub fn remove(id: Uuid) -> Result<(), String> {
    match entry(id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(_) => Err("Cannot remove OS credential".into()),
    }
}
