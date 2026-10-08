//! `aval-svm dataset-synth`: token-permission transactions built locally and simulated with the
//! real engine over an in-memory source (no network), using the SPL Token / Token-2022 programs
//! bundled in LiteSVM. Covers effects that are rare in a mainnet sample: approvals (bounded and
//! unlimited), authority changes, closes and plain token theft. Same record schema as
//! `aval-svm dataset`, with `"source": "synthetic"` and the scenario in `case`.
use crate::cache::Cache;
use crate::dataset::{build_record, tx_meta, EffectRecord};
use crate::decode::{decode, Encoding};
use crate::engine::Engine;
use crate::pool::Pool;
use crate::project::{TOKEN_2022_PROGRAM, TOKEN_PROGRAM};
use crate::source::MemSource;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use solana_account::Account;
use solana_address::Address;
use solana_hash::Hash;
use solana_instruction::{AccountMeta, Instruction};
use solana_message::{Message, VersionedMessage};
use solana_transaction::versioned::VersionedTransaction;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Case {
    ApproveUnlimited,
    ApproveBounded,
    SetOwner,
    SetCloseAuthority,
    CloseToStranger,
    TransferToStranger,
    BenignTransfer,
    BenignApprove,
    BenignClose,
    /// A real transfer plus a permission change in the same transaction.
    BundledTransferApprove,
    BundledTransferSetOwner,
    BundledTransferClose,
}

pub const CASES: [Case; 12] = [
    Case::ApproveUnlimited, Case::ApproveBounded, Case::SetOwner, Case::SetCloseAuthority, Case::CloseToStranger,
    Case::TransferToStranger, Case::BenignTransfer, Case::BenignApprove, Case::BenignClose,
    Case::BundledTransferApprove, Case::BundledTransferSetOwner, Case::BundledTransferClose,
];

impl Case {
    pub fn name(self) -> &'static str {
        match self {
            Case::ApproveUnlimited => "approve_unlimited",
            Case::ApproveBounded => "approve_bounded",
            Case::SetOwner => "set_authority_owner",
            Case::SetCloseAuthority => "set_authority_close",
            Case::CloseToStranger => "close_to_stranger",
            Case::TransferToStranger => "transfer_to_stranger",
            Case::BenignTransfer => "benign_transfer",
            Case::BenignApprove => "benign_approve",
            Case::BenignClose => "benign_close",
            Case::BundledTransferApprove => "bundled_transfer_approve",
            Case::BundledTransferSetOwner => "bundled_transfer_set_owner",
            Case::BundledTransferClose => "bundled_transfer_close",
        }
    }
}

/// splitmix64: small, seeded, reproducible across platforms.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng { Rng(seed) }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 { lo + self.next_u64() % (hi - lo + 1) }
    pub fn key(&mut self) -> Address {
        let mut b = [0u8; 32];
        for c in b.chunks_mut(8) { c.copy_from_slice(&self.next_u64().to_le_bytes()); }
        Address::from(b)
    }
}

pub const TOKEN_ACCOUNT_RENT: u64 = 2_039_280;
pub const MINT_RENT: u64 = 1_461_600;

/// The accounts to seed and the transaction to simulate for one case.
pub struct Scenario {
    pub case: Case,
    pub token_program: Address,
    pub user: Address,
    pub user_token: Address,
    pub mint: Address,
    pub counterparty: Address,
    /// Receiver of the hidden permission in bundled cases.
    pub stranger: Address,
    pub amount: u64,
    pub decimals: u8,
    pub accounts: Vec<(Address, Account)>,
    pub tx: VersionedTransaction,
}

/// Initialized mint (82 bytes): no mint/freeze authority needed for these cases.
pub fn mint_account(program: Address, decimals: u8, supply: u64) -> Account {
    let mut d = vec![0u8; 82];
    d[36..44].copy_from_slice(&supply.to_le_bytes());
    d[44] = decimals;
    d[45] = 1;
    Account { lamports: MINT_RENT, data: d, owner: program, executable: false, rent_epoch: 0 }
}

/// Initialized token account (165 bytes, the base layout both programs accept).
pub fn token_account(program: Address, mint: Address, owner: Address, amount: u64) -> Account {
    let mut d = vec![0u8; 165];
    d[0..32].copy_from_slice(mint.as_ref());
    d[32..64].copy_from_slice(owner.as_ref());
    d[64..72].copy_from_slice(&amount.to_le_bytes());
    d[108] = 1;
    Account { lamports: TOKEN_ACCOUNT_RENT, data: d, owner: program, executable: false, rent_epoch: 0 }
}

fn wallet(lamports: u64) -> Account {
    Account { lamports, owner: solana_sdk_ids::system_program::id(), ..Account::default() }
}

// SPL Token instruction tags (identical in Token-2022).
const APPROVE_CHECKED: u8 = 13;
const SET_AUTHORITY: u8 = 6;
const CLOSE_ACCOUNT: u8 = 9;
const TRANSFER_CHECKED: u8 = 12;
const AUTHORITY_ACCOUNT_OWNER: u8 = 2;
const AUTHORITY_CLOSE_ACCOUNT: u8 = 3;

/// ApproveChecked names the mint, so its decimals are in the simulated state.
fn approve_checked(program: Address, source: Address, mint: Address, delegate: Address, owner: Address, amount: u64, decimals: u8) -> Instruction {
    let mut data = vec![APPROVE_CHECKED];
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    Instruction::new_with_bytes(program, &data, vec![AccountMeta::new(source, false), AccountMeta::new_readonly(mint, false),
        AccountMeta::new_readonly(delegate, false), AccountMeta::new_readonly(owner, true)])
}

fn set_authority(program: Address, account: Address, owner: Address, kind: u8, new: Address) -> Instruction {
    let mut data = vec![SET_AUTHORITY, kind, 1];
    data.extend_from_slice(new.as_ref());
    Instruction::new_with_bytes(program, &data, vec![AccountMeta::new(account, false), AccountMeta::new_readonly(owner, true)])
}

fn close_account(program: Address, account: Address, destination: Address, owner: Address) -> Instruction {
    Instruction::new_with_bytes(program, &[CLOSE_ACCOUNT], vec![AccountMeta::new(account, false), AccountMeta::new(destination, false), AccountMeta::new_readonly(owner, true)])
}

fn transfer_checked(program: Address, source: Address, mint: Address, dest: Address, owner: Address, amount: u64, decimals: u8) -> Instruction {
    let mut data = vec![TRANSFER_CHECKED];
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    Instruction::new_with_bytes(program, &data, vec![AccountMeta::new(source, false), AccountMeta::new_readonly(mint, false), AccountMeta::new(dest, false), AccountMeta::new_readonly(owner, true)])
}

pub fn scenario(case: Case, rng: &mut Rng) -> Scenario {
    let program = Address::from_str(if rng.next_u64().is_multiple_of(2) { TOKEN_PROGRAM } else { TOKEN_2022_PROGRAM }).unwrap();
    let (user, user_token, mint, counterparty) = (rng.key(), rng.key(), rng.key(), rng.key());
    let decimals = [0u8, 2, 6, 6, 8, 9][rng.range(0, 5) as usize];
    let unit = 10u64.pow(u32::from(decimals));
    let balance = rng.range(1, 50_000) * unit + rng.range(0, unit.max(1) - 1);
    // Part of the balance, at least one base unit.
    let amount = rng.range(1, balance);
    let closing = matches!(case, Case::CloseToStranger | Case::BenignClose);
    let mut accounts = vec![
        (user, wallet(rng.range(10_000_000, 5_000_000_000))),
        (mint, mint_account(program, decimals, balance.saturating_mul(4))),
        (user_token, token_account(program, mint, user, if closing { 0 } else { balance })),
    ];
    let stranger = rng.key();
    let transfer_to_counterparty = |accounts: &mut Vec<(Address, Account)>, rng: &mut Rng, amount: u64| {
        let dest = rng.key();
        accounts.push((dest, token_account(program, mint, counterparty, rng.range(0, balance))));
        transfer_checked(program, user_token, mint, dest, user, amount, decimals)
    };
    let (ixs, amount) = match case {
        Case::ApproveUnlimited => (vec![approve_checked(program, user_token, mint, counterparty, user, u64::MAX, decimals)], u64::MAX),
        Case::ApproveBounded | Case::BenignApprove => (vec![approve_checked(program, user_token, mint, counterparty, user, amount, decimals)], amount),
        Case::SetOwner => (vec![set_authority(program, user_token, user, AUTHORITY_ACCOUNT_OWNER, counterparty)], 0),
        Case::SetCloseAuthority => (vec![set_authority(program, user_token, user, AUTHORITY_CLOSE_ACCOUNT, counterparty)], 0),
        Case::CloseToStranger => (vec![close_account(program, user_token, counterparty, user)], 0),
        Case::BenignClose => (vec![close_account(program, user_token, user, user)], 0),
        Case::TransferToStranger | Case::BenignTransfer => (vec![transfer_to_counterparty(&mut accounts, rng, amount)], amount),
        Case::BundledTransferApprove => {
            let t = transfer_to_counterparty(&mut accounts, rng, amount);
            (vec![t, approve_checked(program, user_token, mint, stranger, user, u64::MAX, decimals)], amount)
        }
        Case::BundledTransferSetOwner => {
            let t = transfer_to_counterparty(&mut accounts, rng, amount);
            (vec![t, set_authority(program, user_token, user, AUTHORITY_ACCOUNT_OWNER, stranger)], amount)
        }
        // Drain the whole balance, then close the emptied account with the rent going elsewhere.
        Case::BundledTransferClose => {
            let t = transfer_to_counterparty(&mut accounts, rng, balance);
            (vec![t, close_account(program, user_token, stranger, user)], balance)
        }
    };
    let blockhash = Hash::new_from_array(rng.key().to_bytes());
    let msg = Message::new_with_blockhash(&ixs, Some(&user), &blockhash);
    let tx = VersionedTransaction { signatures: vec![Default::default()], message: VersionedMessage::Legacy(msg) };
    Scenario { case, token_program: program, user, user_token, mint, counterparty, stranger, amount, decimals, accounts, tx }
}

/// Records for `count` scenarios (cases in rotation) and how many failed to simulate.
pub async fn generate(count: usize, seed: u64) -> anyhow::Result<(Vec<EffectRecord>, usize)> {
    let mut rng = Rng::new(seed);
    let engine = Engine::new(Cache::new(MemSource::new(1), Duration::from_secs(3600)), Pool::new(2, 1000));
    let (mut out, mut failed) = (Vec::with_capacity(count), 0usize);
    for i in 0..count {
        let sc = scenario(CASES[i % CASES.len()], &mut rng);
        for (k, a) in &sc.accounts { engine.cache().source().insert(*k, a.clone()); }
        let raw = bincode::serialize(&sc.tx)?;
        let d = decode(&B64.encode(&raw), Encoding::Base64)?;
        let report = engine.simulate(d, true).await?;
        match build_record(&report, tx_meta(&sc.tx), 0, None) {
            Some(mut rec) => {
                rec.source = "synthetic".into();
                rec.case = Some(sc.case.name().into());
                out.push(rec);
            }
            None => failed += 1,
        }
    }
    Ok((out, failed))
}

pub async fn run(count: usize, out: &Path, seed: u64) -> anyhow::Result<()> {
    let (records, failed) = generate(count, seed).await?;
    if let Some(dir) = out.parent() { std::fs::create_dir_all(dir)?; }
    let mut f = std::fs::File::create(out)?;
    let mut per_case = std::collections::BTreeMap::<String, usize>::new();
    for r in &records {
        writeln!(f, "{}", serde_json::to_string(r)?)?;
        *per_case.entry(r.case.clone().unwrap_or_default()).or_default() += 1;
    }
    println!("dataset-synth: {} records ({failed} failed simulations) → {}", records.len(), out.display());
    for (c, n) in per_case { println!("  {c}: {n}"); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::project;

    async fn sim(case: Case, seed: u64) -> (Scenario, EffectRecord) {
        let mut rng = Rng::new(seed);
        let sc = scenario(case, &mut rng);
        let src = MemSource::new(1);
        for (k, a) in &sc.accounts { src.insert(*k, a.clone()); }
        let engine = Engine::new(Cache::new(src, Duration::from_secs(60)), Pool::new(1, 100));
        let raw = bincode::serialize(&sc.tx).unwrap();
        let d = decode(&B64.encode(&raw), Encoding::Base64).unwrap();
        let report = engine.simulate(d, true).await.unwrap();
        assert!(report.outcome.err.is_none(), "{case:?}: {:?} {:?}", report.outcome.err, report.outcome.logs);
        let p = project(&report.pre, &report.outcome.post);
        let rec = build_record(&report, tx_meta(&sc.tx), 0, None).unwrap_or_else(|| panic!("{case:?} not kept: {p:?}"));
        (sc, rec)
    }

    fn authority(rec: &EffectRecord, field: &str) -> Option<(Option<String>, Option<String>)> {
        rec.projection.authority.iter().find(|a| a.field == field).map(|a| (a.pre.clone(), a.post.clone()))
    }

    #[tokio::test]
    async fn approvals_set_a_delegate_with_the_amount() {
        for seed in [1, 2] {
            let (sc, rec) = sim(Case::ApproveUnlimited, seed).await;
            assert_eq!(authority(&rec, "delegate"), Some((None, Some(sc.counterparty.to_string()))));
            assert_eq!(rec.token_accounts[&sc.user_token.to_string()].delegated_amount, Some(u64::MAX.to_string()));
            let (sc, rec) = sim(Case::ApproveBounded, seed).await;
            assert_eq!(rec.token_accounts[&sc.user_token.to_string()].delegated_amount, Some(sc.amount.to_string()));
            assert!(sc.amount < u64::MAX);
            let (sc, rec) = sim(Case::BenignApprove, seed).await;
            assert_eq!(authority(&rec, "delegate"), Some((None, Some(sc.counterparty.to_string()))));
        }
    }

    #[tokio::test]
    async fn set_authority_changes_owner_or_close_authority() {
        let (sc, rec) = sim(Case::SetOwner, 3).await;
        assert_eq!(authority(&rec, "owner"), Some((Some(sc.user.to_string()), Some(sc.counterparty.to_string()))));
        let (sc, rec) = sim(Case::SetCloseAuthority, 3).await;
        assert_eq!(authority(&rec, "closeAuthority"), Some((None, Some(sc.counterparty.to_string()))));
    }

    #[tokio::test]
    async fn close_sends_the_rent_to_the_destination() {
        let (sc, rec) = sim(Case::CloseToStranger, 4).await;
        assert_eq!(rec.projection.closed, vec![sc.user_token.to_string()]);
        let gain = rec.projection.sol.iter().find(|d| d.account == sc.counterparty.to_string()).unwrap();
        assert_eq!(gain.post - gain.pre, TOKEN_ACCOUNT_RENT);
        let (sc, rec) = sim(Case::BenignClose, 4).await;
        assert_eq!(rec.projection.closed, vec![sc.user_token.to_string()]);
        let back = rec.projection.sol.iter().find(|d| d.account == sc.user.to_string()).unwrap();
        assert_eq!(back.post + rec.fee - back.pre, TOKEN_ACCOUNT_RENT);
    }

    #[tokio::test]
    async fn transfers_move_the_users_tokens() {
        for case in [Case::TransferToStranger, Case::BenignTransfer] {
            let (sc, rec) = sim(case, 5).await;
            let out = rec.projection.tokens.iter().find(|t| t.owner == sc.user.to_string()).unwrap();
            let into = rec.projection.tokens.iter().find(|t| t.owner == sc.counterparty.to_string()).unwrap();
            let moved = out.pre.parse::<u64>().unwrap() - out.post.parse::<u64>().unwrap();
            assert_eq!(moved, sc.amount);
            assert_eq!(into.post.parse::<u64>().unwrap() - into.pre.parse::<u64>().unwrap(), sc.amount);
            assert_eq!(out.decimals, Some(sc.decimals));
        }
    }

    #[test]
    fn both_token_programs_are_used_and_seeds_repeat() {
        let mut rng = Rng::new(42);
        let programs: std::collections::HashSet<String> = (0..20).map(|_| scenario(Case::ApproveBounded, &mut rng).token_program.to_string()).collect();
        assert_eq!(programs, [TOKEN_PROGRAM.to_string(), TOKEN_2022_PROGRAM.to_string()].into_iter().collect());
        let a = scenario(Case::SetOwner, &mut Rng::new(7));
        let b = scenario(Case::SetOwner, &mut Rng::new(7));
        assert_eq!((a.user, a.mint, a.amount), (b.user, b.mint, b.amount));
    }

    #[tokio::test]
    async fn approvals_carry_the_mint_decimals() {
        for case in [Case::ApproveUnlimited, Case::ApproveBounded, Case::BenignApprove] {
            let (sc, rec) = sim(case, 8).await;
            assert_eq!(rec.token_accounts[&sc.user_token.to_string()].decimals, Some(sc.decimals), "{case:?}");
        }
    }

    #[tokio::test]
    async fn bundled_cases_transfer_and_hide_a_permission_change() {
        for seed in [9, 10] {
            for case in [Case::BundledTransferApprove, Case::BundledTransferSetOwner, Case::BundledTransferClose] {
                let (sc, rec) = sim(case, seed).await;
                let out = rec.projection.tokens.iter().find(|t| t.owner == sc.user.to_string()).unwrap_or_else(|| panic!("{case:?}"));
                let moved = out.pre.parse::<u64>().unwrap() - out.post.parse::<u64>().unwrap();
                assert_eq!(moved, sc.amount, "{case:?}");
                assert!(rec.projection.tokens.iter().any(|t| t.owner == sc.counterparty.to_string()), "{case:?}");
                match case {
                    Case::BundledTransferApprove => assert_eq!(authority(&rec, "delegate"), Some((None, Some(sc.stranger.to_string())))),
                    Case::BundledTransferSetOwner => assert_eq!(authority(&rec, "owner"), Some((Some(sc.user.to_string()), Some(sc.stranger.to_string())))),
                    _ => {
                        assert_eq!(rec.projection.closed, vec![sc.user_token.to_string()]);
                        let gain = rec.projection.sol.iter().find(|d| d.account == sc.stranger.to_string()).unwrap();
                        assert_eq!(gain.post - gain.pre, TOKEN_ACCOUNT_RENT);
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn generate_rotates_cases_and_marks_records_synthetic() {
        let (records, failed) = generate(24, 42).await.unwrap();
        assert_eq!((records.len(), failed), (24, 0));
        assert!(records.iter().all(|r| r.source == "synthetic"));
        let cases: std::collections::HashSet<_> = records.iter().filter_map(|r| r.case.clone()).collect();
        assert_eq!(cases.len(), CASES.len());
        let digests: std::collections::HashSet<_> = records.iter().map(|r| r.tx_digest.clone()).collect();
        assert_eq!(digests.len(), 24);
    }
}
