use solana_account::Account;
use solana_address::Address;
use solana_message::VersionedMessage;
use solana_sdk_ids::{address_lookup_table, bpf_loader_upgradeable};
use std::collections::{HashMap, HashSet};

pub const LOOKUP_TABLE_META_SIZE: usize = 56;
const PROGRAM_TAG: [u8; 4] = [2, 0, 0, 0];
const PROGRAMDATA_TAG: [u8; 4] = [3, 0, 0, 0];

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum GatherError {
    #[error("address lookup table {0} not found")]
    TableNotFound(Address),
    #[error("address lookup table {0} is invalid")]
    TableInvalid(Address),
    #[error("lookup index {index} out of range for table {table}")]
    IndexOutOfRange { table: Address, index: u8 },
}

fn push_unique(out: &mut Vec<Address>, seen: &mut HashSet<Address>, k: Address) {
    if seen.insert(k) {
        out.push(k);
    }
}

pub fn first_pass(msg: &VersionedMessage) -> Vec<Address> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for k in msg.static_account_keys() {
        push_unique(&mut out, &mut seen, *k);
    }
    for l in msg.address_table_lookups().unwrap_or(&[]) {
        push_unique(&mut out, &mut seen, l.account_key);
    }
    out
}

fn table_addresses(table: Address, acct: &Account) -> Result<Vec<Address>, GatherError> {
    if acct.owner != address_lookup_table::id() || acct.data.len() < LOOKUP_TABLE_META_SIZE {
        return Err(GatherError::TableInvalid(table));
    }
    let body = &acct.data[LOOKUP_TABLE_META_SIZE..];
    if !body.len().is_multiple_of(32) {
        return Err(GatherError::TableInvalid(table));
    }
    Ok(body.as_chunks::<32>().0.iter().map(|c| Address::from(*c)).collect())
}

/// Loaded addresses in message order: all writable lookups first, then all readonly.
pub fn resolve_lookups(
    msg: &VersionedMessage,
    accounts: &HashMap<Address, Option<Account>>,
) -> Result<Vec<Address>, GatherError> {
    let lookups = msg.address_table_lookups().unwrap_or(&[]);
    let mut tables = Vec::with_capacity(lookups.len());
    for l in lookups {
        let acct = accounts
            .get(&l.account_key)
            .and_then(|a| a.as_ref())
            .ok_or(GatherError::TableNotFound(l.account_key))?;
        tables.push(table_addresses(l.account_key, acct)?);
    }
    let pick = |table: Address, addrs: &Vec<Address>, i: u8| {
        addrs.get(i as usize).copied().ok_or(GatherError::IndexOutOfRange { table, index: i })
    };
    let mut out = Vec::new();
    for (l, addrs) in lookups.iter().zip(&tables) {
        for &i in &l.writable_indexes {
            out.push(pick(l.account_key, addrs, i)?);
        }
    }
    for (l, addrs) in lookups.iter().zip(&tables) {
        for &i in &l.readonly_indexes {
            out.push(pick(l.account_key, addrs, i)?);
        }
    }
    Ok(out)
}

pub fn programdata_of(acct: &Account) -> Option<Address> {
    if acct.owner != bpf_loader_upgradeable::id() || acct.data.len() < 36 || acct.data[..4] != PROGRAM_TAG {
        return None;
    }
    Some(Address::from(<[u8; 32]>::try_from(&acct.data[4..36]).unwrap()))
}

pub fn is_programdata(acct: &Account) -> bool {
    acct.owner == bpf_loader_upgradeable::id() && acct.data.len() >= 4 && acct.data[..4] == PROGRAMDATA_TAG
}

/// LiteSVM loads a program when its account is set, and needs ProgramData present first.
pub fn vm_order(accounts: Vec<(Address, Option<Account>)>) -> Vec<(Address, Option<Account>)> {
    let rank = |a: &Option<Account>| match a {
        Some(a) if is_programdata(a) => 0,
        Some(a) if a.executable => 2,
        _ => 1,
    };
    let mut v = accounts;
    v.sort_by_key(|(_, a)| rank(a));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_hash::Hash;
    use solana_message::{v0, MessageHeader, compiled_instruction::CompiledInstruction};
    use solana_message::v0::MessageAddressTableLookup;

    fn key(n: u8) -> Address { Address::from([n; 32]) }

    fn table_account(addresses: &[Address]) -> Account {
        let mut data = vec![0u8; LOOKUP_TABLE_META_SIZE];
        data[0] = 1; // ProgramState::LookupTable
        for a in addresses { data.extend_from_slice(a.as_ref()); }
        Account { lamports: 1, data, owner: address_lookup_table::id(), executable: false, rent_epoch: 0 }
    }

    fn v0_msg(lookups: Vec<MessageAddressTableLookup>) -> VersionedMessage {
        VersionedMessage::V0(v0::Message {
            header: MessageHeader { num_required_signatures: 1, num_readonly_signed_accounts: 0, num_readonly_unsigned_accounts: 1 },
            account_keys: vec![key(1), key(2)],
            recent_blockhash: Hash::default(),
            instructions: vec![CompiledInstruction { program_id_index: 1, accounts: vec![0], data: vec![] }],
            address_table_lookups: lookups,
        })
    }

    #[test]
    fn first_pass_includes_static_keys_and_tables() {
        let msg = v0_msg(vec![MessageAddressTableLookup { account_key: key(9), writable_indexes: vec![0], readonly_indexes: vec![] }]);
        assert_eq!(first_pass(&msg), vec![key(1), key(2), key(9)]);
    }

    #[test]
    fn resolves_writable_then_readonly() {
        let msg = v0_msg(vec![MessageAddressTableLookup { account_key: key(9), writable_indexes: vec![1], readonly_indexes: vec![0] }]);
        let accounts = HashMap::from([(key(9), Some(table_account(&[key(20), key(21)])))]);
        assert_eq!(resolve_lookups(&msg, &accounts).unwrap(), vec![key(21), key(20)]);
    }

    #[test]
    fn writable_across_all_tables_precede_readonly() {
        let msg = v0_msg(vec![
            MessageAddressTableLookup { account_key: key(9), writable_indexes: vec![0], readonly_indexes: vec![1] },
            MessageAddressTableLookup { account_key: key(10), writable_indexes: vec![0], readonly_indexes: vec![1] },
        ]);
        let accounts = HashMap::from([
            (key(9), Some(table_account(&[key(20), key(21)]))),
            (key(10), Some(table_account(&[key(30), key(31)]))),
        ]);
        assert_eq!(resolve_lookups(&msg, &accounts).unwrap(), vec![key(20), key(30), key(21), key(31)]);
    }

    #[test]
    fn missing_or_bad_tables_are_errors() {
        let msg = v0_msg(vec![MessageAddressTableLookup { account_key: key(9), writable_indexes: vec![5], readonly_indexes: vec![] }]);
        let none = HashMap::from([(key(9), None)]);
        assert_eq!(resolve_lookups(&msg, &none), Err(GatherError::TableNotFound(key(9))));
        let short = HashMap::from([(key(9), Some(table_account(&[key(20)])))]);
        assert_eq!(resolve_lookups(&msg, &short), Err(GatherError::IndexOutOfRange { table: key(9), index: 5 }));
    }

    #[test]
    fn finds_programdata_of_upgradeable_program() {
        let mut data = PROGRAM_TAG.to_vec();
        data.extend_from_slice(key(7).as_ref());
        let program = Account { lamports: 1, data, owner: bpf_loader_upgradeable::id(), executable: true, rent_epoch: 0 };
        assert_eq!(programdata_of(&program), Some(key(7)));
        let plain = Account { owner: key(3), ..program.clone() };
        assert_eq!(programdata_of(&plain), None);
    }

    #[test]
    fn vm_order_puts_programdata_first_and_programs_last() {
        let pd = Account { lamports: 1, data: PROGRAMDATA_TAG.to_vec(), owner: bpf_loader_upgradeable::id(), executable: false, rent_epoch: 0 };
        let prog = Account { executable: true, ..Account::default() };
        let plain = Account::default();
        let ordered = vm_order(vec![(key(1), Some(prog)), (key(2), Some(plain)), (key(3), Some(pd)), (key(4), None)]);
        let keys: Vec<_> = ordered.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys[0], key(3));
        assert_eq!(*keys.last().unwrap(), key(1));
    }
}
