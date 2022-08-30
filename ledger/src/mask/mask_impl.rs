use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use mina_hasher::Fp;
use mina_signer::CompressedPubKey;

use crate::{
    account::{Account, AccountId, TokenId},
    address::{Address, AddressIterator},
    base::{AccountIndex, BaseLedger, GetOrCreated, Uuid},
    tree::{Database, DatabaseError},
    tree_version::{TreeVersion, V2},
};

use super::Mask;

pub(super) enum MaskImpl {
    Root {
        database: Database<V2>,
        childs: HashMap<Uuid, Mask>,
    },
    Attached {
        parent: Option<Mask>,
        owning_account: HashMap<AccountIndex, Account>,
        token_to_account: HashMap<TokenId, AccountId>,
        id_to_addr: HashMap<AccountId, Address>,
        last_location: Option<Address>,
        first_location_in_mask: Option<Address>,
        depth: u8,
        naccounts: usize,
        childs: HashMap<Uuid, Mask>,
        uuid: Uuid,
    },
}

impl std::fmt::Debug for MaskImpl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Root { database, childs } => f
                .debug_struct("Root")
                .field("database", &database.get_uuid())
                .field("childs", childs)
                .finish(),
            Self::Attached {
                parent,
                owning_account,
                token_to_account,
                id_to_addr,
                last_location,
                first_location_in_mask,
                depth,
                naccounts,
                childs,
                uuid,
            } => f
                .debug_struct("Attached")
                .field("uuid", uuid)
                .field("parent", &parent.as_ref().map(|p| p.get_uuid()))
                .field("owning_account", &owning_account.len())
                .field("token_to_account", &token_to_account.len())
                .field("id_to_addr", &id_to_addr.len())
                .field("last_location", last_location)
                .field("first_location_in_mask", first_location_in_mask)
                .field("depth", depth)
                .field("naccounts", naccounts)
                .field("childs", &childs.len())
                .finish(),
        }
    }
}

impl MaskImpl {
    pub fn is_attached(&self) -> bool {
        match self {
            MaskImpl::Root { .. } => false,
            MaskImpl::Attached { parent, .. } => parent.is_some(),
        }
    }

    pub fn set_parent(&mut self, mask: &Mask) {
        match self {
            MaskImpl::Root { .. } => panic!("set_parent() on a root"),
            MaskImpl::Attached { parent, .. } => {
                assert!(parent.is_none(), "mask is already attached");
                *parent = Some(mask.clone());
            }
        }
    }

    fn uuid(&self) -> Uuid {
        self.get_uuid()
    }

    pub fn get_parent(&self) -> Option<Mask> {
        match self {
            MaskImpl::Root { .. } => None,
            MaskImpl::Attached { parent, .. } => parent.clone(),
        }
    }

    pub fn unset_parent(&mut self, trigger_detach_signal: bool) {
        let parent = self.remove_parent();

        assert!(
            parent.is_some(),
            "unset_parent called on a non-attached mask"
        );

        if trigger_detach_signal {
            // TODO: Async.Ivar.fill_if_empty t.detached_parent_signal () ;
        }
    }

    /// get hash from mask, if present, else from its parent
    pub fn get_hash(&self, addr: Address) -> Option<Fp> {
        self.get_inner_hash_at_addr(addr).ok()
    }

    /// commit all state to the parent, flush state locally
    pub fn commit(&mut self) {
        let depth = self.depth() as usize;
        let self_uuid = self.uuid();
        let old_root_hash = self.merkle_root();

        match self {
            MaskImpl::Root { .. } => panic!("commit on a root"),
            MaskImpl::Attached {
                parent,
                owning_account,
                token_to_account,
                id_to_addr,
                ..
            } => {
                let parent = parent.as_mut().expect("Mask doesn't have parent");
                assert_ne!(parent.get_uuid(), self_uuid);

                let accounts = {
                    token_to_account.clear();
                    id_to_addr.clear();
                    std::mem::take(owning_account)
                };

                for (index, account) in accounts {
                    let addr = Address::from_index(index.clone(), depth);
                    parent.set_impl(addr, account, Some(self_uuid));
                }

                // Parent merkle root after committing should be the same as the \
                // old one in the mask
                assert_eq!(old_root_hash, parent.merkle_root());
            }
        }
    }

    /// called when parent sets an account; update local state
    ///
    /// if the mask's parent sets an account, we can prune an entry in the mask
    /// if the account in the parent is the same in the mask *)
    pub fn parent_set_notify(&mut self, account: &Account) {
        assert!(self.is_attached());

        match self {
            MaskImpl::Root { .. } => panic!("parent_set_notify on a root"),
            MaskImpl::Attached {
                owning_account,
                id_to_addr,
                ..
            } => {
                let account_id = account.id();

                let own_account = match {
                    id_to_addr
                        .get(&account_id)
                        .and_then(|addr| owning_account.get(&addr.to_index()))
                        .cloned()
                } {
                    Some(own) => own,
                    None => return,
                };

                if own_account != *account {
                    // Do not delete our account if it is different than the parent one
                    return;
                }

                self.remove_own_account(&[account_id]);
            }
        }
    }

    pub fn children(&self) -> Vec<Mask> {
        let childs = match self {
            MaskImpl::Root { childs, .. } => childs,
            MaskImpl::Attached { childs, .. } => childs,
        };

        childs.values().cloned().collect()
    }

    pub fn remove_parent(&mut self) -> Option<Mask> {
        match self {
            MaskImpl::Root { .. } => panic!(),
            MaskImpl::Attached { parent, .. } => parent.take(),
        }
    }

    pub fn remove_child(&mut self, child: &Mask) -> Option<Mask> {
        let childs = match self {
            MaskImpl::Root { childs, .. } => childs,
            MaskImpl::Attached { childs, .. } => childs,
        };

        let uuid = child.get_uuid();
        childs.remove(&uuid)
    }

    pub fn depth(&self) -> u8 {
        match self {
            MaskImpl::Root { database, .. } => database.depth(),
            MaskImpl::Attached { depth, .. } => *depth,
        }
    }

    fn emulate_tree_to_get_hash(&self) -> Fp {
        let tree_depth = self.depth() as usize;
        let naccounts = self.num_accounts();
        let mut account_index = 0;

        self.emulate_recursive(0, tree_depth, &mut account_index, naccounts as u64)
    }

    fn emulate_tree_to_get_hash_at(&self, addr: Address) -> Fp {
        let tree_depth = self.depth() as usize;

        let current_depth = addr.length();

        let mut children = addr.iter_children(tree_depth);
        let naccounts = children.len();

        // First child
        let mut account_index = children.next().unwrap().to_index().0 as u64;

        self.emulate_recursive(
            current_depth,
            tree_depth,
            &mut account_index,
            naccounts as u64,
        )
    }

    fn emulate_recursive(
        &self,
        current_depth: usize,
        tree_depth: usize,
        account_index: &mut u64,
        naccounts: u64,
    ) -> Fp {
        if current_depth == tree_depth {
            let account_addr = Address::from_index(AccountIndex(*account_index), tree_depth);
            let account = match self.get(account_addr) {
                Some(account) => account,
                None => return V2::empty_hash_at_depth(0),
            };

            *account_index += 1;
            return account.hash();
        }

        let left_hash =
            self.emulate_recursive(current_depth + 1, tree_depth, account_index, naccounts);
        let right_hash = if *account_index < naccounts {
            self.emulate_recursive(current_depth + 1, tree_depth, account_index, naccounts)
        } else {
            V2::empty_hash_at_depth(tree_depth - current_depth)
        };

        V2::hash_node(tree_depth - current_depth, left_hash, right_hash)
    }

    fn remove_own_account(&mut self, ids: &[AccountId]) {
        match self {
            MaskImpl::Root { .. } => todo!(),
            MaskImpl::Attached {
                owning_account,
                token_to_account,
                id_to_addr,
                last_location,
                first_location_in_mask,
                naccounts,
                ..
            } => {
                let mut addrs = ids
                    .iter()
                    .map(|account_id| id_to_addr.remove(&account_id).unwrap())
                    .collect::<Vec<_>>();
                addrs.sort_by(|a, b| a.to_index().cmp(&b.to_index()));

                for addr in addrs.iter().rev() {
                    let account = owning_account.remove(&addr.to_index()).unwrap();
                    token_to_account.remove(&account.token_id).unwrap();

                    if last_location
                        .as_ref()
                        .map(|last| last == addr)
                        .unwrap_or(false)
                    {
                        *last_location = addr.prev();
                    }

                    if first_location_in_mask
                        .as_ref()
                        .map(|first| first == addr)
                        .unwrap_or(false)
                    {
                        *last_location = None;
                        *first_location_in_mask = None;
                    }

                    *naccounts -= 1;
                }
            }
        }
    }

    pub(super) fn set_impl(
        &mut self,
        addr: Address,
        account: Account,
        child_to_ignore: Option<Uuid>,
    ) {
        let childs = match self {
            MaskImpl::Root { childs, .. } => childs,
            MaskImpl::Attached { childs, .. } => childs,
        };

        for (uuid, child) in childs {
            if Some(*uuid) == child_to_ignore {
                continue;
            }
            child.parent_set_notify(&account)
        }

        if let MaskImpl::Root { database, .. } = self {
            return database.set(addr, account);
        };

        let existing = self.get(addr.clone()).is_some();

        match self {
            MaskImpl::Root { database, .. } => database.set(addr, account),
            MaskImpl::Attached {
                owning_account,
                token_to_account,
                id_to_addr,
                last_location,
                first_location_in_mask,
                naccounts,
                ..
            } => {
                let account_index: AccountIndex = addr.to_index();
                let account_id = account.id();
                let token_id = account.token_id.clone();

                owning_account.insert(account_index, account);
                id_to_addr.insert(account_id.clone(), addr.clone());
                token_to_account.insert(token_id, account_id);

                if !existing {
                    *naccounts += 1;
                }

                if last_location
                    .as_ref()
                    .map(|l| l.to_index() < addr.to_index())
                    .unwrap_or(true)
                {
                    *last_location = Some(addr.clone());
                }

                if first_location_in_mask
                    .as_ref()
                    .map(|l| l.to_index() > addr.to_index())
                    .unwrap_or(true)
                {
                    *first_location_in_mask = Some(addr);
                }
            }
        }
    }

    /// For tests only, check if the address is in the mask, without checking parent
    #[cfg(test)]
    pub fn test_is_in_mask(&self, addr: &Address) -> bool {
        match self {
            MaskImpl::Root { database, .. } => database.get(addr.clone()).is_some(),
            MaskImpl::Attached { owning_account, .. } => {
                let index = addr.to_index();
                owning_account.contains_key(&index)
            }
        }
    }
}

impl BaseLedger for MaskImpl {
    fn to_list(&self) -> Vec<Account> {
        let depth = self.depth() as usize;
        let num_accounts = self.num_accounts();

        let mut accounts = Vec::with_capacity(num_accounts);

        for index in 0..num_accounts {
            let index = AccountIndex(index as u64);
            let addr = Address::from_index(index, depth);
            accounts.push(self.get(addr).unwrap_or_else(|| Account::empty()));
        }

        accounts
    }

    fn iter<F>(&self, fun: F)
    where
        F: FnMut(&Account),
    {
        let accounts = self.to_list();
        accounts.iter().for_each(fun)
    }

    fn fold<B, F>(&self, init: B, fun: F) -> B
    where
        F: FnMut(B, &Account) -> B,
    {
        let accounts = self.to_list();
        accounts.iter().fold(init, fun)
    }

    fn fold_with_ignored_accounts<B, F>(
        &self,
        ignoreds: HashSet<AccountId>,
        init: B,
        mut fun: F,
    ) -> B
    where
        F: FnMut(B, &Account) -> B,
    {
        let accounts = self.to_list();
        accounts.iter().fold(init, |accum, account| {
            if !ignoreds.contains(&account.id()) {
                fun(accum, account)
            } else {
                accum
            }
        })
    }

    fn fold_until<B, F>(&self, init: B, mut fun: F) -> B
    where
        F: FnMut(B, &Account) -> std::ops::ControlFlow<B, B>,
    {
        use std::ops::ControlFlow::*;

        let accounts = self.to_list();
        let mut accum = init;

        for account in &accounts {
            match fun(accum, account) {
                Continue(acc) => accum = acc,
                Break(acc) => {
                    accum = acc;
                    break;
                }
            }
        }

        accum
    }

    fn accounts(&self) -> HashSet<AccountId> {
        self.to_list()
            .into_iter()
            .map(|account| account.id())
            .collect()
    }

    fn token_owner(&self, token_id: TokenId) -> Option<AccountId> {
        let (parent, token_to_account) = match self {
            MaskImpl::Root { database, .. } => return database.token_owner(token_id),
            MaskImpl::Attached {
                parent,
                token_to_account,
                ..
            } => (parent, token_to_account),
        };

        if let Some(account_id) = token_to_account.get(&token_id).cloned() {
            return Some(account_id);
        };

        parent.as_ref()?.token_owner(token_id)
    }

    fn token_owners(&self) -> HashSet<AccountId> {
        // TODO: Not sure if it's the correct impl
        self.to_list()
            .into_iter()
            .map(|account| account.id())
            .collect()
    }

    fn tokens(&self, public_key: CompressedPubKey) -> HashSet<TokenId> {
        let mut set = HashSet::with_capacity(1024);

        for account in self.to_list() {
            if account.public_key == public_key {
                set.insert(account.token_id);
            }
        }

        set
    }

    fn location_of_account(&self, account_id: &AccountId) -> Option<Address> {
        let (parent, id_to_addr) = match self {
            MaskImpl::Root { database, .. } => return database.location_of_account(account_id),
            MaskImpl::Attached {
                parent, id_to_addr, ..
            } => (parent, id_to_addr),
        };

        if let Some(addr) = id_to_addr.get(&account_id).cloned() {
            return Some(addr);
        }

        parent.as_ref()?.location_of_account(account_id)
    }

    fn location_of_account_batch(
        &self,
        account_ids: &[AccountId],
    ) -> Vec<(AccountId, Option<Address>)> {
        account_ids
            .iter()
            .map(|account_id| {
                let addr = self.location_of_account(account_id);
                (account_id.clone(), addr)
            })
            .collect()
    }

    fn get_or_create_account(
        &mut self,
        account_id: AccountId,
        account: Account,
    ) -> Result<GetOrCreated, DatabaseError> {
        if let Some(addr) = self.location_of_account(&account_id) {
            return Ok(GetOrCreated::Existed(addr));
        }

        if let MaskImpl::Root { database, .. } = self {
            return database.get_or_create_account(account_id, account);
        };

        let last_filled = self.last_filled();

        match self {
            MaskImpl::Root { database, .. } => database.get_or_create_account(account_id, account),
            MaskImpl::Attached {
                owning_account,
                token_to_account,
                id_to_addr,
                last_location,
                first_location_in_mask,
                depth,
                naccounts,
                ..
            } => {
                let location = match last_filled {
                    Some(last) => last.next().ok_or(DatabaseError::OutOfLeaves).unwrap(),
                    None => Address::first(*depth as usize),
                };

                let account_index: AccountIndex = location.to_index();
                let token_id = account.token_id.clone();

                id_to_addr.insert(account_id.clone(), location.clone());
                *last_location = Some(location.clone());
                token_to_account.insert(token_id, account_id);
                owning_account.insert(account_index, account);
                *naccounts += 1;

                if first_location_in_mask.is_none() {
                    *first_location_in_mask = Some(location.clone());
                }

                Ok(GetOrCreated::Added(location))
            }
        }
    }

    fn close(self) {
        // Drop
    }

    fn last_filled(&self) -> Option<Address> {
        match self {
            MaskImpl::Root { database, .. } => database.last_filled(),
            MaskImpl::Attached {
                parent,
                last_location,
                ..
            } => {
                let last_filled_parent = match parent.as_ref().and_then(|p| p.last_filled()) {
                    Some(last) => last,
                    None => return last_location.clone(),
                };

                let last_filled = match last_location {
                    Some(last) => last,
                    None => return Some(last_filled_parent),
                };

                let last_filled_parent_index = last_filled_parent.to_index();
                let last_filled_index = last_filled.to_index();

                if last_filled_index > last_filled_parent_index {
                    Some(last_filled.clone())
                } else {
                    Some(last_filled_parent)
                }
            }
        }
    }

    fn get_uuid(&self) -> Uuid {
        match self {
            MaskImpl::Root { database, .. } => database.get_uuid(),
            MaskImpl::Attached { uuid, .. } => *uuid,
        }
    }

    fn get_directory(&self) -> Option<PathBuf> {
        None
    }

    fn get(&self, addr: Address) -> Option<Account> {
        let (parent, owning_account) = match self {
            MaskImpl::Root { database, .. } => return database.get(addr),
            MaskImpl::Attached {
                parent,
                owning_account,
                ..
            } => (parent, owning_account),
        };

        let account_index = addr.to_index();
        if let Some(account) = owning_account.get(&account_index).cloned() {
            return Some(account);
        }

        parent.as_ref()?.get(addr)
    }

    fn get_batch(&self, addr: &[Address]) -> Vec<(Address, Option<Account>)> {
        addr.iter()
            .map(|addr| (addr.clone(), self.get(addr.clone())))
            .collect()
    }

    fn set(&mut self, addr: Address, account: Account) {
        self.set_impl(addr, account, None)
    }

    fn set_batch(&mut self, list: &[(Address, Account)]) {
        for (addr, account) in list {
            self.set(addr.clone(), account.clone())
        }
    }

    fn get_at_index(&self, index: AccountIndex) -> Option<Account> {
        let addr = Address::from_index(index, self.depth() as usize);
        self.get(addr)
    }

    fn set_at_index(&mut self, index: AccountIndex, account: Account) -> Result<(), ()> {
        let addr = Address::from_index(index, self.depth() as usize);
        self.set(addr, account);
        Ok(())
    }

    fn index_of_account(&self, account_id: AccountId) -> Option<AccountIndex> {
        let (parent, id_to_addr) = match self {
            MaskImpl::Root { database, .. } => return database.index_of_account(account_id),
            MaskImpl::Attached {
                parent, id_to_addr, ..
            } => (parent, id_to_addr),
        };

        if let Some(addr) = id_to_addr.get(&account_id).cloned() {
            return Some(addr.to_index());
        };

        parent.as_ref()?.index_of_account(account_id)
    }

    fn merkle_root(&self) -> Fp {
        self.emulate_tree_to_get_hash()
    }

    fn merkle_path(&self, addr: Address) -> AddressIterator {
        addr.into_iter()
    }

    fn merkle_path_at_index(&self, index: AccountIndex) -> Option<AddressIterator> {
        let addr = Address::from_index(index, self.depth() as usize);
        Some(addr.into_iter())
    }

    fn remove_accounts(&mut self, ids: &[AccountId]) {
        match self {
            MaskImpl::Root { database, .. } => database.remove_accounts(ids),
            MaskImpl::Attached {
                parent, id_to_addr, ..
            } => {
                let (mask_keys, parent_keys): (Vec<_>, Vec<_>) = ids
                    .iter()
                    .cloned()
                    .partition(|id| id_to_addr.contains_key(id));

                if !parent_keys.is_empty() {
                    let parent = parent.as_mut().expect("Parent missing");
                    parent.remove_accounts(&parent_keys);
                }

                self.remove_own_account(&mask_keys);
            }
        }
    }

    fn detached_signal(&mut self) {
        todo!()
    }

    fn depth(&self) -> u8 {
        self.depth()
    }

    fn num_accounts(&self) -> usize {
        self.last_filled()
            .map(|addr| addr.to_index().0 as usize + 1)
            .unwrap_or(0)
        // self.with(|this| this.naccounts)
    }

    fn merkle_path_at_addr(&self, addr: Address) -> Option<AddressIterator> {
        Some(addr.into_iter())
    }

    fn get_inner_hash_at_addr(&self, addr: Address) -> Result<Fp, ()> {
        let self_depth = self.depth() as usize;

        if addr.length() > self_depth {
            return Err(());
        }

        Ok(self.emulate_tree_to_get_hash_at(addr))
    }

    fn set_inner_hash_at_addr(&mut self, _addr: Address, _hash: Fp) -> Result<(), ()> {
        todo!()
    }

    fn set_all_accounts_rooted_at(
        &mut self,
        addr: Address,
        accounts: &[Account],
    ) -> Result<(), ()> {
        let self_depth = self.depth() as usize;

        if addr.length() > self_depth {
            return Err(());
        }

        for (child_addr, account) in addr.iter_children(self_depth).zip(accounts) {
            self.set(child_addr, account.clone());
        }

        Ok(())
    }

    fn get_all_accounts_rooted_at(&self, addr: Address) -> Option<Vec<(Address, Account)>> {
        let self_depth = self.depth() as usize;

        if addr.length() > self_depth {
            return None;
        }

        let children = addr.iter_children(self_depth);
        let mut accounts = Vec::with_capacity(children.len());

        for child_addr in children {
            let account = match self.get(child_addr.clone()) {
                Some(account) => account,
                None => continue,
            };
            accounts.push((child_addr, account));
        }

        if accounts.is_empty() {
            None
        } else {
            Some(accounts)
        }
    }

    fn make_space_for(&mut self, _space: usize) {
        // No op, we're in memory
    }
}
