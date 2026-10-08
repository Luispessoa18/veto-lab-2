use serde::Serialize;
use solana_account::Account;
use solana_address::Address;
use solana_transaction_error::TransactionError;
use std::collections::{HashMap, HashSet};

pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const TOKEN_2022_PROGRAM: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

#[derive(Debug, Serialize, PartialEq)]
pub struct SolDelta { pub account: String, pub pre: u64, pub post: u64 }

#[derive(Debug, Serialize, PartialEq)]
pub struct TokenDelta {
    pub account: String, pub mint: String, pub owner: String,
    pub pre: String, pub post: String, pub decimals: Option<u8>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct AuthorityChange { pub account: String, pub field: String, pub pre: Option<String>, pub post: Option<String> }

#[derive(Debug, Serialize, Default, PartialEq)]
pub struct Projection {
    pub sol: Vec<SolDelta>, pub tokens: Vec<TokenDelta>, pub authority: Vec<AuthorityChange>,
    pub closed: Vec<String>, pub created: Vec<String>,
}

type TokenField = (&'static str, fn(&TokenView) -> Option<Address>);

struct TokenView { mint: Address, owner: Address, amount: u64, delegate: Option<Address>, close_authority: Option<Address> }

fn addr(b: &[u8]) -> Address { Address::from(<[u8; 32]>::try_from(b).unwrap()) }
fn coption(d: &[u8], tag_at: usize) -> Option<Address> {
    (d[tag_at] == 1).then(|| addr(&d[tag_at + 4..tag_at + 36]))
}

fn is_token_program(owner: &Address) -> bool {
    let o = owner.to_string();
    o == TOKEN_PROGRAM || o == TOKEN_2022_PROGRAM
}

/// SPL token account layout (165 bytes; Token-2022 adds AccountType=2 at byte 165).
fn token_view(a: &Account) -> Option<TokenView> {
    let d = &a.data;
    let is_account = d.len() == 165 || (d.len() > 165 && d[165] == 2);
    if !is_token_program(&a.owner) || !is_account || d[108] == 0 {
        return None;
    }
    Some(TokenView {
        mint: addr(&d[0..32]),
        owner: addr(&d[32..64]),
        amount: u64::from_le_bytes(d[64..72].try_into().unwrap()),
        delegate: coption(d, 72),
        close_authority: coption(d, 129),
    })
}

/// Token owner and delegate of an initialized SPL Token / Token-2022 token account.
pub fn token_authorities(a: &Account) -> Option<(Address, Option<Address>)> {
    token_view(a).map(|t| (t.owner, t.delegate))
}

/// Mint layout: decimals at byte 44.
fn decimals_of(mint: &Address, pre: &HashMap<Address, Option<Account>>) -> Option<u8> {
    let m = pre.get(mint)?.as_ref()?;
    (is_token_program(&m.owner) && m.data.len() >= 82).then(|| m.data[44])
}

pub fn project(pre: &HashMap<Address, Option<Account>>, post: &HashMap<Address, Account>) -> Projection {
    let mut keys: Vec<&Address> = post.keys().collect();
    keys.sort_by_key(|k| k.to_string());
    let mut p = Projection::default();
    for k in keys {
        let after = &post[k];
        let before = pre.get(k).and_then(|a| a.as_ref());
        let pre_l = before.map_or(0, |a| a.lamports);
        let ks = k.to_string();
        if pre_l != after.lamports {
            p.sol.push(SolDelta { account: ks.clone(), pre: pre_l, post: after.lamports });
        }
        if pre_l == 0 && after.lamports > 0 { p.created.push(ks.clone()); }
        if pre_l > 0 && after.lamports == 0 { p.closed.push(ks.clone()); }
        if let (Some(b), Some(a)) = (before, Some(after)) {
            if b.owner != a.owner {
                p.authority.push(AuthorityChange { account: ks.clone(), field: "programOwner".into(), pre: Some(b.owner.to_string()), post: Some(a.owner.to_string()) });
            }
        }
        let tb = before.and_then(token_view);
        let ta = token_view(after);
        let (pre_amt, post_amt) = (tb.as_ref().map_or(0, |t| t.amount), ta.as_ref().map_or(0, |t| t.amount));
        if let Some(t) = ta.as_ref().or(tb.as_ref()) {
            if pre_amt != post_amt {
                p.tokens.push(TokenDelta { account: ks.clone(), mint: t.mint.to_string(), owner: t.owner.to_string(), pre: pre_amt.to_string(), post: post_amt.to_string(), decimals: decimals_of(&t.mint, pre) });
            }
        }
        let fields: [TokenField; 3] = [
            ("owner", |t| Some(t.owner)),
            ("delegate", |t| t.delegate),
            ("closeAuthority", |t| t.close_authority),
        ];
        if tb.is_some() || ta.is_some() {
            for (name, get) in fields {
                let (b, a) = (tb.as_ref().and_then(get), ta.as_ref().and_then(get));
                if b != a && tb.is_some() && ta.is_some() {
                    p.authority.push(AuthorityChange { account: ks.clone(), field: name.into(), pre: b.map(|x| x.to_string()), post: a.map(|x| x.to_string()) });
                }
            }
        }
    }
    p
}

/// One simulated world, as the divergence rule sees it.
pub struct WorldView<'a> {
    pub err: Option<&'a TransactionError>,
    pub projection: &'a Projection,
}

/// Error kind: the variant, and for an instruction error the inner variant (not the index or
/// a custom code's value).
fn error_kind(e: &TransactionError) -> String {
    match e {
        TransactionError::InstructionError(_, ie) => format!("InstructionError::{:?}", std::mem::discriminant(ie)),
        e => format!("{:?}", std::mem::discriminant(e)),
    }
}

/// Two deltas differ by more than `bps` of the larger one (0 apart never does; zero in one
/// world and non-zero in the other always does).
fn beyond(p: i128, s: i128, bps: u64) -> bool {
    let apart = (p - s).abs();
    if apart == 0 {
        return false;
    }
    if p == 0 || s == 0 {
        return true;
    }
    apart * 10_000 > p.abs().max(s.abs()) * bps as i128
}

type AuthorityKey<'a> = (&'a str, &'a str, Option<&'a str>, Option<&'a str>);

fn authority_set(v: &[AuthorityChange]) -> HashSet<AuthorityKey<'_>> {
    v.iter().map(|a| (a.account.as_str(), a.field.as_str(), a.pre.as_deref(), a.post.as_deref())).collect()
}

/// Do two worlds differ in what matters to the user? Yes when the outcome (or its error kind)
/// differs, any authority / ownership change or created / closed set differs, or a signer's SOL
/// delta or a signer-owned/delegated token account's delta (`user_tokens`) differs beyond
/// `tolerance_bps`. Third-party accounts moving differently is not divergence by itself.
pub fn divergent(p: WorldView, s: WorldView, signers: &HashSet<String>, user_tokens: &HashSet<String>, tolerance_bps: u64) -> bool {
    if p.err.map(error_kind) != s.err.map(error_kind) {
        return true;
    }
    let (pp, sp) = (p.projection, s.projection);
    let set = |v: &Vec<String>| v.iter().cloned().collect::<HashSet<_>>();
    if authority_set(&pp.authority) != authority_set(&sp.authority) || set(&pp.created) != set(&sp.created) || set(&pp.closed) != set(&sp.closed) {
        return true;
    }
    let sol = |pr: &Projection, k: &String| pr.sol.iter().find(|d| &d.account == k).map_or(0, |d| d.post as i128 - d.pre as i128);
    let tok = |pr: &Projection, k: &String| pr.tokens.iter().find(|d| &d.account == k)
        .map_or(0, |d| d.post.parse::<i128>().unwrap_or(0) - d.pre.parse::<i128>().unwrap_or(0));
    signers.iter().any(|k| beyond(sol(pp, k), sol(sp, k), tolerance_bps))
        || user_tokens.iter().any(|k| beyond(tok(pp, k), tok(sp, k), tolerance_bps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;
    use std::collections::HashSet;

    fn key(n: u8) -> Address { Address::from([n; 32]) }
    fn token_account(mint: Address, owner: Address, amount: u64, delegate: Option<Address>) -> Account {
        let mut d = vec![0u8; 165];
        d[0..32].copy_from_slice(mint.as_ref());
        d[32..64].copy_from_slice(owner.as_ref());
        d[64..72].copy_from_slice(&amount.to_le_bytes());
        if let Some(del) = delegate { d[72] = 1; d[76..108].copy_from_slice(del.as_ref()); }
        d[108] = 1; // initialized
        Account { lamports: 2_039_280, data: d, owner: Address::from_str(TOKEN_PROGRAM).unwrap(), executable: false, rent_epoch: 0 }
    }

    #[test]
    fn token_transfer_and_new_delegate_are_projected() {
        let (acct, mint, owner, thief) = (key(1), key(2), key(3), key(9));
        let pre = HashMap::from([(acct, Some(token_account(mint, owner, 5_000_000, None)))]);
        let post = HashMap::from([(acct, token_account(mint, owner, 0, Some(thief)))]);
        let p = project(&pre, &post);
        assert_eq!(p.tokens, vec![TokenDelta { account: acct.to_string(), mint: mint.to_string(), owner: owner.to_string(), pre: "5000000".into(), post: "0".into(), decimals: None }]);
        assert_eq!(p.authority, vec![AuthorityChange { account: acct.to_string(), field: "delegate".into(), pre: None, post: Some(thief.to_string()) }]);
        assert!(p.sol.is_empty());
    }

    #[test]
    fn created_and_closed_accounts() {
        let pre = HashMap::from([(key(1), None), (key(2), Some(Account { lamports: 10, ..Account::default() }))]);
        let post = HashMap::from([(key(1), Account { lamports: 7, ..Account::default() }), (key(2), Account::default())]);
        let p = project(&pre, &post);
        assert_eq!(p.created, vec![key(1).to_string()]);
        assert_eq!(p.closed, vec![key(2).to_string()]);
        assert_eq!(p.sol.len(), 2);
    }

    fn sol(account: Address, pre: u64, post: u64) -> SolDelta { SolDelta { account: account.to_string(), pre, post } }
    fn tok(account: Address, pre: u64, post: u64) -> TokenDelta {
        TokenDelta { account: account.to_string(), mint: key(40).to_string(), owner: key(1).to_string(), pre: pre.to_string(), post: post.to_string(), decimals: None }
    }
    fn view<'a>(err: Option<&'a solana_transaction_error::TransactionError>, projection: &'a Projection) -> WorldView<'a> {
        WorldView { err, projection }
    }
    fn users() -> (HashSet<String>, HashSet<String>) {
        (HashSet::from([key(1).to_string()]), HashSet::from([key(10).to_string()]))
    }
    /// The signer pays 5000 and swaps SOL for `out` units in its token account key(10);
    /// the pool key(20) moves by `pool`.
    fn swap(out: u64, pool: u64) -> Projection {
        Projection {
            sol: vec![sol(key(1), 10_000_000, 9_995_000), sol(key(20), 1_000_000, 1_000_000 + pool)],
            tokens: vec![tok(key(10), 0, out), tok(key(21), 1_000_000_000, 1_000_000_000 - out)],
            ..Projection::default()
        }
    }

    #[test]
    fn identical_worlds_are_not_divergent() {
        let (signers, tokens) = users();
        let p = swap(1_000_000, 7);
        assert!(!divergent(view(None, &p), view(None, &swap(1_000_000, 7)), &signers, &tokens, 50));
    }

    #[test]
    fn a_pool_only_difference_is_not_divergent() {
        let (signers, tokens) = users();
        let (p, mut s) = (swap(1_000_000, 7), swap(1_000_000, 9_999));
        s.tokens[1] = tok(key(21), 3_000_000_000, 2_999_000_000);
        assert!(!divergent(view(None, &p), view(None, &s), &signers, &tokens, 50));
    }

    #[test]
    fn user_output_beyond_the_tolerance_is_divergent() {
        let (signers, tokens) = users();
        let p = swap(1_000_000, 7);
        // 0.3% (30 bps) and 0.2% (20 bps) apart.
        let (s3, s2) = (swap(997_000, 7), swap(998_000, 7));
        assert!(divergent(view(None, &p), view(None, &s3), &signers, &tokens, 25));
        assert!(!divergent(view(None, &p), view(None, &s2), &signers, &tokens, 25));
        // Default 50 bps: 0.6% is divergent, 0.3% is not.
        assert!(divergent(view(None, &p), view(None, &swap(994_000, 7)), &signers, &tokens, 50));
        assert!(!divergent(view(None, &p), view(None, &s3), &signers, &tokens, 50));
    }

    #[test]
    fn zero_in_one_world_only_is_divergent() {
        let (signers, tokens) = users();
        let p = swap(1, 7);
        let mut s = swap(1, 7);
        s.tokens.remove(0);
        assert!(divergent(view(None, &p), view(None, &s), &signers, &tokens, 10_000));
    }

    #[test]
    fn signer_sol_delta_beyond_the_tolerance_is_divergent() {
        let (signers, tokens) = users();
        let p = swap(1_000_000, 7);
        let mut s = swap(1_000_000, 7);
        s.sol[0] = sol(key(1), 10_000_000, 9_000_000);
        assert!(divergent(view(None, &p), view(None, &s), &signers, &tokens, 50));
    }

    #[test]
    fn one_failing_world_or_another_error_kind_is_divergent() {
        use solana_instruction::error::InstructionError;
        use solana_transaction_error::TransactionError as E;
        let (signers, tokens) = users();
        let p = swap(1_000_000, 7);
        let failed = Projection::default();
        let custom = |n| E::InstructionError(0, InstructionError::Custom(n));
        assert!(divergent(view(None, &p), view(Some(&custom(1)), &failed, ), &signers, &tokens, 50));
        assert!(divergent(view(Some(&custom(1)), &failed), view(Some(&E::AccountNotFound), &failed), &signers, &tokens, 50));
        assert!(divergent(view(Some(&custom(1)), &failed), view(Some(&E::InstructionError(0, InstructionError::InvalidAccountData)), &failed), &signers, &tokens, 50));
        assert!(!divergent(view(Some(&custom(1)), &failed), view(Some(&custom(2)), &failed), &signers, &tokens, 50), "same kind");
    }

    #[test]
    fn an_authority_or_lifecycle_change_in_one_world_only_is_divergent() {
        let (signers, tokens) = users();
        let p = swap(1_000_000, 7);
        let mut s = swap(1_000_000, 7);
        s.authority.push(AuthorityChange { account: key(30).to_string(), field: "delegate".into(), pre: None, post: Some(key(9).to_string()) });
        assert!(divergent(view(None, &p), view(None, &s), &signers, &tokens, 50));
        let mut c = swap(1_000_000, 7);
        c.created.push(key(31).to_string());
        assert!(divergent(view(None, &p), view(None, &c), &signers, &tokens, 50), "created sets differ");
    }
}
