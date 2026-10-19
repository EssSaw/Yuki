use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub const SERVICE_NAME: &str = "yuki-9router";
pub const USER_NAME: &str = "chat_api_key";

pub trait SecretStore: Send + Sync {
    fn get_api_key(&self, service: &str, user: &str) -> Result<Option<String>, String>;
    fn set_api_key(&self, service: &str, user: &str, key: &str) -> Result<(), String>;
    fn delete_api_key(&self, service: &str, user: &str) -> Result<(), String>;
}

pub struct KeyringSecretStore;

impl SecretStore for KeyringSecretStore {
    fn get_api_key(&self, service: &str, user: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(service, user)
            .map_err(|e| format!("Failed to access keyring: {e}"))?;
        match entry.get_password() {
            Ok(pwd) => Ok(Some(pwd)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Failed to retrieve secret from keyring: {e}")),
        }
    }

    fn set_api_key(&self, service: &str, user: &str, key: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(service, user)
            .map_err(|e| format!("Failed to access keyring: {e}"))?;
        entry
            .set_password(key)
            .map_err(|e| format!("Failed to save secret to keyring: {e}"))
    }

    fn delete_api_key(&self, service: &str, user: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(service, user)
            .map_err(|e| format!("Failed to access keyring: {e}"))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("Failed to delete secret from keyring: {e}")),
        }
    }
}

#[derive(Default)]
pub struct MockSecretStore {
    secrets: RwLock<HashMap<(String, String), String>>,
}

impl MockSecretStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
}

impl SecretStore for MockSecretStore {
    fn get_api_key(&self, service: &str, user: &str) -> Result<Option<String>, String> {
        let map = self.secrets.read().map_err(|e| e.to_string())?;
        Ok(map.get(&(service.to_string(), user.to_string())).cloned())
    }

    fn set_api_key(&self, service: &str, user: &str, key: &str) -> Result<(), String> {
        let mut map = self.secrets.write().map_err(|e| e.to_string())?;
        map.insert((service.to_string(), user.to_string()), key.to_string());
        Ok(())
    }

    fn delete_api_key(&self, service: &str, user: &str) -> Result<(), String> {
        let mut map = self.secrets.write().map_err(|e| e.to_string())?;
        map.remove(&(service.to_string(), user.to_string()));
        Ok(())
    }
}
