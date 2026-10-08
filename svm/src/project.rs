use serde::Serialize;
use solana_account::Account;
use solana_address::Address;
use std::collections::HashMap;

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

/// Mint and owner of an initialized SPL token account, as strings.
pub fn token_mint_owner(a: &Account) -> Option<(String, String)> {
    token_view(a).map(|t| (t.mint.to_string(), t.owner.to_string()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

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
    fn token_mint_owner_reads_token_accounts_only() {
        let (mint, owner) = (key(2), key(3));
        assert_eq!(token_mint_owner(&token_account(mint, owner, 1, None)), Some((mint.to_string(), owner.to_string())));
        assert_eq!(token_mint_owner(&Account { lamports: 1, ..Account::default() }), None);
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
}
