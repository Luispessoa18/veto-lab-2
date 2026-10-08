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
    assert_eq!(first.min_slot, 500);
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

async fn sim(e: &Engine<MemSource>, raw: &[u8]) -> Result<aval_svm::engine::SimReport, EngineError> {
    e.simulate(decode(&B64.encode(raw), Encoding::Base64).unwrap(), false).await
}

fn wallet(lamports: u64) -> Account {
    Account { lamports, owner: solana_sdk_ids::system_program::id(), ..Account::default() }
}

#[tokio::test]
async fn without_dissent_there_is_one_world() {
    let e = engine();
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let r = sim(&e, &raw).await.unwrap();
    assert_eq!((r.worlds, r.divergent), (1, false));
    assert!(r.alternate.is_none());
}

#[tokio::test]
async fn dissent_on_a_signer_refuses() {
    let e = engine();
    e.cache().source().set_dissent(key(1), Some(wallet(1)));
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let err = sim(&e, &raw).await.unwrap_err();
    assert!(matches!(err, EngineError::Upstream(_)), "{err}");
    assert_eq!(err.to_string(), format!("upstream unavailable: upstreams disagree on {}", key(1)));
}

#[tokio::test]
async fn dissent_on_a_token_account_the_signer_owns_or_delegates_refuses() {
    for (owner, delegate) in [(key(1), None), (key(8), Some(key(1)))] {
        let e = engine();
        e.cache().source().insert(key(3), common::token_account(key(4), owner, 5, delegate));
        e.cache().source().set_dissent(key(3), Some(common::token_account(key(4), owner, 0, delegate)));
        let raw = common::transfer_with_readonly(key(1), key(2), 1_000_000, key(3));
        let err = sim(&e, &raw).await.unwrap_err();
        assert!(err.to_string().contains(&format!("upstreams disagree on {}", key(3))), "{err}");
    }
}

#[tokio::test]
async fn dissent_on_a_third_party_token_account_is_tolerated() {
    let e = engine();
    e.cache().source().insert(key(3), common::token_account(key(4), key(8), 5, None));
    e.cache().source().set_dissent(key(3), Some(common::token_account(key(4), key(8), 0, None)));
    let raw = common::transfer_with_readonly(key(1), key(2), 1_000_000, key(3));
    assert_eq!(sim(&e, &raw).await.unwrap().worlds, 2);
}

#[tokio::test]
async fn dissent_on_an_executable_in_either_view_refuses() {
    let e = engine();
    e.cache().source().insert(key(5), wallet(777));
    e.cache().source().set_dissent(key(5), Some(Account { executable: true, ..wallet(777) }));
    let raw = common::transfer_with_readonly(key(1), key(2), 1_000_000, key(5));
    let err = sim(&e, &raw).await.unwrap_err();
    assert!(err.to_string().contains(&format!("upstreams disagree on {}", key(5))), "{err}");
}

#[tokio::test]
async fn tolerated_dissent_with_matching_worlds_is_not_divergent() {
    let e = engine();
    e.cache().source().insert(key(5), wallet(777));
    e.cache().source().set_dissent(key(5), Some(wallet(778)));
    let raw = common::transfer_with_readonly(key(1), key(2), 1_000_000, key(5));
    let r = sim(&e, &raw).await.unwrap();
    assert!(r.outcome.err.is_none(), "{:?}", r.outcome.logs);
    assert_eq!((r.worlds, r.divergent), (2, false));
    let alt = r.alternate.as_ref().expect("world S");
    assert!(alt.outcome.err.is_none());
    assert_eq!(r.pre[&key(5)].as_ref().unwrap().lamports, 777, "world P's state is reported");
    assert_eq!(alt.pre[&key(5)].as_ref().unwrap().lamports, 778);
}

#[tokio::test]
async fn a_failing_secondary_world_is_the_answer() {
    let e = engine();
    // World S runs with a rent of 10_000 lamports/byte: 1_000_000 no longer makes key(2) rent-exempt.
    let rent = solana_rent::Rent { lamports_per_byte: 10_000, ..solana_rent::Rent::default() };
    e.cache().source().set_dissent(solana_sdk_ids::sysvar::rent::id(), Some(Account {
        lamports: 1, data: bincode::serialize(&rent).unwrap(), owner: solana_sdk_ids::sysvar::id(), ..Account::default()
    }));
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let r = sim(&e, &raw).await.unwrap();
    assert!(matches!(r.outcome.err, Some(solana_transaction_error::TransactionError::InsufficientFundsForRent { .. })), "{:?}", r.outcome.err);
    assert_eq!((r.worlds, r.divergent), (2, true));
    assert!(r.alternate.as_ref().unwrap().outcome.err.is_none(), "the other world (P) succeeded");
}

#[tokio::test]
async fn differing_post_states_are_divergent() {
    let e = engine();
    e.cache().source().set_dissent(key(2), Some(wallet(5_000_000)));
    let (_, raw) = transfer_tx(key(1), key(2), 1_000_000);
    let r = sim(&e, &raw).await.unwrap();
    assert!(r.outcome.err.is_none());
    assert_eq!((r.worlds, r.divergent), (2, true));
    assert_eq!(r.outcome.post[&key(2)].lamports, 1_000_000, "world P's outcome");
    assert_eq!(r.alternate.as_ref().unwrap().outcome.post[&key(2)].lamports, 6_000_000);
}

#[tokio::test]
async fn dissent_on_a_lookup_table_refuses() {
    let e = engine();
    e.cache().source().insert(key(9), lookup_table(&[key(2)]));
    e.cache().source().set_dissent(key(9), Some(lookup_table(&[key(3)])));
    let err = sim(&e, &v0_transfer_via_table()).await.unwrap_err();
    assert!(err.to_string().contains(&format!("upstreams disagree on {}", key(9))), "{err}");
}
