//! The real compiled aval_registry program, run in LiteSVM.
use aval_svm::registry_client::*;
use litesvm::LiteSVM;
use sha2::{Digest, Sha256};
use solana_instruction::error::InstructionError;
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;
use solana_transaction_error::TransactionError;

fn vm() -> LiteSVM {
    let mut svm = LiteSVM::new();
    svm.add_program(program_id(), include_bytes!("fixtures/aval_registry.so")).unwrap();
    svm
}

fn funded(svm: &mut LiteSVM) -> Keypair {
    let kp = Keypair::new();
    svm.airdrop(&kp.pubkey(), 10_000_000_000).unwrap();
    kp
}

fn send(svm: &mut LiteSVM, kp: &Keypair, ix: solana_instruction::Instruction) -> Result<(), TransactionError> {
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&kp.pubkey()), &[kp], svm.latest_blockhash());
    svm.send_transaction(tx).map(|_| ()).map_err(|f| f.err)
}

fn custom(e: &TransactionError) -> Option<u32> {
    match e { TransactionError::InstructionError(_, InstructionError::Custom(c)) => Some(*c), _ => None }
}

#[test]
fn fixture_matches_pinned_hash() {
    let pinned = include_str!("fixtures/aval_registry.so.sha256").trim();
    assert_eq!(hex::encode(Sha256::digest(include_bytes!("fixtures/aval_registry.so"))), pinned);
}

#[test]
fn init_then_two_contiguous_batches() {
    let mut svm = vm();
    let kp = funded(&mut svm);
    let auth = kp.pubkey();
    send(&mut svm, &kp, init_registry_ix(&auth)).unwrap();
    let reg = registry_pda(&auth);
    send(&mut svm, &kp, anchor_batch_ix(&auth, &reg, 0, [1; 32], 0, 3)).unwrap();
    send(&mut svm, &kp, anchor_batch_ix(&auth, &reg, 1, [2; 32], 3, 2)).unwrap();
    let r = decode_registry(&svm.get_account(&reg).unwrap().data).unwrap();
    assert_eq!((r.authority, r.next_batch, r.next_record, r.last_root), (auth, 2, 5, [2; 32]));
    let b = decode_batch(&svm.get_account(&batch_pda(&reg, 1)).unwrap().data).unwrap();
    assert_eq!((b.registry, b.index, b.root, b.first_record, b.count), (reg, 1, [2; 32], 3, 2));
}

#[test]
fn gaps_replays_empty_batches_and_strangers_are_rejected() {
    let mut svm = vm();
    let kp = funded(&mut svm);
    let auth = kp.pubkey();
    send(&mut svm, &kp, init_registry_ix(&auth)).unwrap();
    let reg = registry_pda(&auth);
    let gap = send(&mut svm, &kp, anchor_batch_ix(&auth, &reg, 0, [1; 32], 7, 1)).unwrap_err();
    assert_eq!(custom(&gap), Some(ERR_NON_CONTIGUOUS));
    let empty = send(&mut svm, &kp, anchor_batch_ix(&auth, &reg, 0, [1; 32], 0, 0)).unwrap_err();
    assert_eq!(custom(&empty), Some(ERR_EMPTY_BATCH));
    send(&mut svm, &kp, anchor_batch_ix(&auth, &reg, 0, [1; 32], 0, 1)).unwrap();
    svm.expire_blockhash();
    let replay = send(&mut svm, &kp, anchor_batch_ix(&auth, &reg, 1, [1; 32], 0, 1)).unwrap_err();
    assert_eq!(custom(&replay), Some(ERR_NON_CONTIGUOUS));
    let stranger = funded(&mut svm);
    let mut ix = anchor_batch_ix(&stranger.pubkey(), &reg, 1, [9; 32], 1, 1);
    ix.accounts[1].pubkey = batch_pda(&reg, 1);
    let bad = send(&mut svm, &stranger, ix).unwrap_err();
    assert_eq!(custom(&bad), Some(ERR_UNAUTHORIZED));
}

#[test]
fn decoders_reject_wrong_accounts() {
    assert!(decode_registry(&[0u8; 10]).is_none());
    assert!(decode_batch(&[0u8; 200]).is_none()); // wrong discriminator
}
