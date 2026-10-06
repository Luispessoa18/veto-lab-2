#![allow(dead_code)]
use solana_address::Address;
use solana_hash::Hash;
use solana_message::{Message, VersionedMessage};
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;

pub fn key(n: u8) -> Address {
    Address::from([n; 32])
}

/// Unsigned legacy SOL transfer; `from` pays the fee.
pub fn transfer_tx(from: Address, to: Address, lamports: u64) -> (VersionedTransaction, Vec<u8>) {
    let ix = solana_system_interface::instruction::transfer(&from, &to, lamports);
    let msg = Message::new_with_blockhash(&[ix], Some(&from), &Hash::new_from_array([7; 32]));
    let tx = VersionedTransaction { signatures: vec![Signature::default()], message: VersionedMessage::Legacy(msg) };
    let raw = bincode::serialize(&tx).unwrap();
    (tx, raw)
}

/// A transfer padded with a memo of `memo_len` bytes, to build oversized transactions.
pub fn padded_tx(from: Address, memo_len: usize) -> Vec<u8> {
    let memo = solana_instruction::Instruction {
        program_id: "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr".parse().unwrap(),
        accounts: vec![],
        data: vec![b'a'; memo_len],
    };
    let ix = solana_system_interface::instruction::transfer(&from, &key(2), 1_000_000);
    let msg = Message::new_with_blockhash(&[ix, memo], Some(&from), &Hash::new_from_array([7; 32]));
    let tx = VersionedTransaction { signatures: vec![Signature::default()], message: VersionedMessage::Legacy(msg) };
    bincode::serialize(&tx).unwrap()
}
