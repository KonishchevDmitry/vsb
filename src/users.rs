use std::cell::RefCell;
use std::collections::{HashMap, hash_map::Entry};

use nix::unistd::{User, Group};

use crate::core::GenericResult;

#[derive(Default)]
pub struct UsersCache {
    users: RefCell<HashMap<String, Option<u32>>>,
    groups: RefCell<HashMap<String, Option<u32>>>,
}

impl UsersCache {
    pub fn new() -> UsersCache {
        return Default::default()
    }

    pub fn get_uid(&self, name: &str) -> GenericResult<Option<u32>> {
        Ok(match self.users.borrow_mut().entry(name.to_owned()) {
            Entry::Vacant(entry) => {
                let user = User::from_name(name).map_err(|e| format!(
                    "Unable to lookup {:?} user: {}", name, e))?;

                *entry.insert(user.map(|user| user.uid.as_raw()))
            },

            Entry::Occupied(entry) => *entry.get(),
        })
    }

    pub fn get_gid(&self, name: &str) -> GenericResult<Option<u32>> {
        Ok(match self.groups.borrow_mut().entry(name.to_owned()) {
            Entry::Vacant(entry) => {
                let group = Group::from_name(name).map_err(|e| format!(
                    "Unable to lookup {:?} group: {}", name, e))?;

                *entry.insert(group.map(|group| group.gid.as_raw()))
            },

            Entry::Occupied(entry) => *entry.get(),
        })
    }
}