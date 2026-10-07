//! Hand-written client for the aval_registry Anchor program. Anchor's wire format:
//! instruction data = sha256("global:<ix>")[..8] ‖ borsh(args); account data =
//! sha256("account:<Name>")[..8] ‖ borsh(fields). Pinned by tests against the
//! real compiled program (tests/registry_program.rs).
use sha2::{Digest, Sha256};
use solana_address::Address;
use solana_instruction::{AccountMeta, Instruction};
use std::str::FromStr;

pub const ERR_UNAUTHORIZED: u32 = 6000;
pub const ERR_NON_CONTIGUOUS: u32 = 6001;
pub const ERR_EMPTY_BATCH: u32 = 6002;

const SYSTEM_PROGRAM: Address = Address::new_from_array([0; 32]);

pub fn program_id() -> Address {
    Address::from_str(include_str!("../tests/fixtures/aval_registry.id").trim()).expect("program id file")
}

fn disc(namespace: &str, name: &str) -> [u8; 8] {
    let h = Sha256::digest(format!("{namespace}:{name}").as_bytes());
    h[..8].try_into().unwrap()
}

pub fn registry_pda(authority: &Address) -> Address {
    Address::find_program_address(&[b"registry", authority.as_ref()], &program_id()).0
}

pub fn batch_pda(registry: &Address, index: u64) -> Address {
    Address::find_program_address(&[b"batch", registry.as_ref(), &index.to_le_bytes()], &program_id()).0
}

pub fn init_registry_ix(authority: &Address) -> Instruction {
    Instruction {
        program_id: program_id(),
        accounts: vec![
            AccountMeta::new(registry_pda(authority), false),
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data: disc("global", "init_registry").to_vec(),
    }
}

pub fn anchor_batch_ix(authority: &Address, registry: &Address, next_batch: u64, root: [u8; 32], first_record: u64, count: u32) -> Instruction {
    let mut data = disc("global", "anchor_batch").to_vec();
    data.extend_from_slice(&root);
    data.extend_from_slice(&first_record.to_le_bytes());
    data.extend_from_slice(&count.to_le_bytes());
    Instruction {
        program_id: program_id(),
        accounts: vec![
            AccountMeta::new(*registry, false),
            AccountMeta::new(batch_pda(registry, next_batch), false),
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(SYSTEM_PROGRAM, false),
        ],
        data,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryAccount { pub authority: Address, pub next_batch: u64, pub next_record: u64, pub last_root: [u8; 32] }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchAccount { pub registry: Address, pub index: u64, pub root: [u8; 32], pub first_record: u64, pub count: u32, pub slot: u64, pub unix_timestamp: i64 }

struct Reader<'a> { d: &'a [u8], at: usize }
impl<'a> Reader<'a> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let s = self.d.get(self.at..self.at + N)?;
        self.at += N;
        s.try_into().ok()
    }
}

pub fn decode_registry(data: &[u8]) -> Option<RegistryAccount> {
    if data.len() < 89 || data[..8] != disc("account", "Registry") { return None; }
    let mut r = Reader { d: data, at: 8 };
    Some(RegistryAccount {
        authority: Address::from(r.take::<32>()?),
        next_batch: u64::from_le_bytes(r.take()?),
        next_record: u64::from_le_bytes(r.take()?),
        last_root: r.take()?,
    })
}

pub fn decode_batch(data: &[u8]) -> Option<BatchAccount> {
    if data.len() < 109 || data[..8] != disc("account", "Batch") { return None; }
    let mut r = Reader { d: data, at: 8 };
    Some(BatchAccount {
        registry: Address::from(r.take::<32>()?),
        index: u64::from_le_bytes(r.take()?),
        root: r.take()?,
        first_record: u64::from_le_bytes(r.take()?),
        count: u32::from_le_bytes(r.take()?),
        slot: u64::from_le_bytes(r.take()?),
        unix_timestamp: i64::from_le_bytes(r.take()?),
    })
}
