mod common;
use aval_svm::cache::Cache;
use aval_svm::decode::{decode, Encoding};
use aval_svm::engine::{Engine, EngineError};
use aval_svm::pool::Pool;
use aval_svm::source::MemSource;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use common::{key, transfer_tx};
use solana_account::Account;
use std::time::Duration;

fn engine() -> Engine<MemSource> {
    let src = MemSource::new(500);
    src.insert(key(1), Account { lamports: 10_000_000_000, owner: solana_sdk_ids::system_program::id(), ..Account::default() });
    Engine::new(Cache::new(src, Duration::from_secs(60)), Pool::new(2, 100))
}

#[tokio::test]
async fn simulates_and_reports_cache_and_digest() {
    let e = engine();
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let first = e.simulate(decode(&B64.encode(&raw), Encoding::Base64).unwrap(), false).await.unwrap();
    assert!(first.outcome.err.is_none(), "{:?}", first.outcome.logs);
    assert_eq!(first.slot, 500);
    assert!(first.misses > 0);
    assert_eq!(first.digest, aval_svm::decode::message_digest(&raw).unwrap());
    let second = e.simulate(decode(&B64.encode(&raw), Encoding::Base64).unwrap(), false).await.unwrap();
    assert_eq!(second.misses, 0);
}

#[tokio::test]
async fn upstream_down_with_cold_cache_is_an_error() {
    let e = engine();
    e.cache().source().set_fail(true);
    let (_, raw) = transfer_tx(key(1), key(2), 1);
    let err = e.simulate(decode(&B64.encode(&raw), Encoding::Base64).unwrap(), false).await.unwrap_err();
    assert!(matches!(err, EngineError::Upstream(_)));
    assert_eq!(err.code(), -32005);
}

#[tokio::test]
async fn uses_the_clusters_rent_sysvar() {
    let e = engine();
    // 1_000_000 covers the default rent-exempt minimum for an empty account (890_880).
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let ok = e.simulate(decode(&B64.encode(&raw), Encoding::Base64).unwrap(), false).await.unwrap();
    assert!(ok.outcome.err.is_none());
    // A cluster whose rent is 10_000 lamports/byte needs 1_280_000.
    let rent = solana_rent::Rent { lamports_per_byte: 10_000, ..solana_rent::Rent::default() };
    e.cache().source().insert(solana_sdk_ids::sysvar::rent::id(), Account {
        lamports: 1, data: bincode::serialize(&rent).unwrap(), owner: solana_sdk_ids::sysvar::id(), ..Account::default()
    });
    let strict = e.simulate(decode(&B64.encode(&raw), Encoding::Base64).unwrap(), true).await.unwrap();
    assert!(matches!(strict.outcome.err, Some(solana_transaction_error::TransactionError::InsufficientFundsForRent { .. })));
}

#[tokio::test]
async fn simulates_with_the_clusters_clock() {
    let e = engine();
    let cluster = solana_clock::Clock { slot: 500, epoch_start_timestamp: 1_699_000_000, epoch: 812, leader_schedule_epoch: 813, unix_timestamp: 1_700_000_123 };
    e.cache().source().insert(solana_sdk_ids::sysvar::clock::id(), Account {
        lamports: 1, data: bincode::serialize(&cluster).unwrap(), owner: solana_sdk_ids::sysvar::id(), ..Account::default()
    });
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let r = e.simulate(decode(&B64.encode(&raw), Encoding::Base64).unwrap(), false).await.unwrap();
    assert!(r.outcome.err.is_none(), "{:?}", r.outcome.logs);
    let c = r.outcome.clock;
    assert_eq!((c.unix_timestamp, c.epoch, c.leader_schedule_epoch, c.epoch_start_timestamp), (1_700_000_123, 812, 813, 1_699_000_000));
    assert!(c.slot >= 500, "slot is max(vm, cluster, input)");
}

/// v0 transfer from key(1) to key(2), where key(2) comes from lookup table key(9).
fn v0_transfer_via_table() -> Vec<u8> {
    use solana_message::{compiled_instruction::CompiledInstruction, v0, v0::MessageAddressTableLookup, MessageHeader, VersionedMessage};
    let data = solana_system_interface::instruction::transfer(&key(1), &key(2), 1_000_000).data;
    let msg = v0::Message {
        header: MessageHeader { num_required_signatures: 1, num_readonly_signed_accounts: 0, num_readonly_unsigned_accounts: 1 },
        account_keys: vec![key(1), solana_sdk_ids::system_program::id()],
        recent_blockhash: solana_hash::Hash::new_from_array([7; 32]),
        instructions: vec![CompiledInstruction { program_id_index: 1, accounts: vec![0, 2], data }],
        address_table_lookups: vec![MessageAddressTableLookup { account_key: key(9), writable_indexes: vec![0], readonly_indexes: vec![] }],
    };
    let tx = solana_transaction::versioned::VersionedTransaction { signatures: vec![Default::default()], message: VersionedMessage::V0(msg) };
    bincode::serialize(&tx).unwrap()
}

fn lookup_table(addresses: &[solana_address::Address]) -> Account {
    let mut data = vec![0u8; aval_svm::gather::LOOKUP_TABLE_META_SIZE];
    data[0] = 1; // ProgramState::LookupTable
    data[4..12].copy_from_slice(&u64::MAX.to_le_bytes()); // active (never deactivated)
    for a in addresses { data.extend_from_slice(a.as_ref()); }
    Account { lamports: 1_000_000_000, data, owner: solana_sdk_ids::address_lookup_table::id(), executable: false, rent_epoch: 0 }
}

#[tokio::test]
async fn a_lookup_table_cached_as_absent_is_refetched_once() {
    let e = engine();
    // The table is read (and cached as absent) before it exists upstream...
    e.cache().get_many(&[key(9)], false).await.unwrap();
    // ...then it is created.
    e.cache().source().insert(key(9), lookup_table(&[key(2)]));
    let raw = v0_transfer_via_table();
    let r = e.simulate(decode(&B64.encode(&raw), Encoding::Base64).unwrap(), false).await.unwrap();
    assert!(r.outcome.err.is_none(), "{:?} {:?}", r.outcome.err, r.outcome.logs);
    assert_eq!(r.outcome.post[&key(2)].lamports, 1_000_000);
}

#[tokio::test]
async fn a_table_that_stays_absent_is_still_an_error() {
    let e = engine();
    let raw = v0_transfer_via_table();
    let err = e.simulate(decode(&B64.encode(&raw), Encoding::Base64).unwrap(), false).await.unwrap_err();
    assert!(matches!(err, EngineError::Gather(aval_svm::gather::GatherError::TableNotFound(_))), "{err}");
    assert_eq!(err.code(), -32602);
}
