use crate::gather::vm_order;
use litesvm::LiteSVM;
use solana_account::Account;
use solana_address::Address;
use solana_clock::Clock;
use solana_hash::Hash;
use solana_message::inner_instruction::InnerInstructionsList;
use solana_sdk_ids::{native_loader, sysvar};
use solana_transaction::versioned::VersionedTransaction;
use solana_transaction_context::transaction::TransactionReturnData;
use solana_transaction_error::TransactionError;
use std::collections::{HashMap, HashSet};
use crate::gather::{is_programdata, programdata_of};
use tokio::sync::oneshot;

pub struct SimInput {
    pub tx: VersionedTransaction,
    pub accounts: Vec<(Address, Option<Account>)>,
    pub slot: u64,
}

#[derive(Debug)]
pub struct SimOutcome {
    pub err: Option<TransactionError>,
    pub logs: Vec<String>,
    pub units: u64,
    pub fee: u64,
    pub inner: InnerInstructionsList,
    pub return_data: TransactionReturnData,
    /// Post-state of every account the transaction wrote (empty when it failed).
    pub post: HashMap<Address, Account>,
    pub blockhash: Hash,
    /// The Clock sysvar the transaction ran with.
    pub clock: Clock,
}

#[derive(Debug, thiserror::Error)]
pub enum SimError {
    #[error("unsupported program {0}")]
    UnsupportedProgram(Address),
    #[error("simulator pool unavailable")]
    PoolDown,
    #[error("simulator panicked")]
    Panicked,
}

pub fn new_vm() -> LiteSVM {
    LiteSVM::new().with_sigverify(false).with_blockhash_check(false)
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// What a VM remembers about a program/ProgramData account it already holds: the full data
/// when small, otherwise its length and first 64 bytes. ProgramData's first 45 bytes carry the
/// deploy slot and upgrade authority, so an upgrade always changes the fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fingerprint {
    Full(Vec<u8>),
    Prefix(usize, [u8; 64]),
}

impl Fingerprint {
    pub fn of(data: &[u8]) -> Fingerprint {
        match data.first_chunk::<64>() {
            Some(prefix) if data.len() > 64 => Fingerprint::Prefix(data.len(), *prefix),
            _ => Fingerprint::Full(data.to_vec()),
        }
    }
}

/// Program/ProgramData accounts already set in a VM, by address.
pub type Loaded = HashMap<Address, Fingerprint>;

/// The cluster's Clock with a slot that never moves the VM backwards and is never behind
/// the state the request was read at.
pub fn cluster_clock(vm: &Clock, cluster: &Clock, input_slot: u64) -> Clock {
    Clock { slot: vm.slot.max(cluster.slot).max(input_slot), ..cluster.clone() }
}

/// Deserializes a sysvar the source returned; `None` when absent, not sysvar-owned or garbled
/// (garbled is logged: the VM's default is used instead).
fn cluster_sysvar<T: serde::de::DeserializeOwned>(accounts: &[(Address, Option<Account>)], id: Address) -> Option<T> {
    let a = accounts.iter().find(|(k, _)| *k == id)?.1.as_ref()?;
    if a.owner != sysvar::id() {
        tracing::warn!("sysvar {id} is not owned by the sysvar program; using the VM default");
        return None;
    }
    match bincode::deserialize::<T>(&a.data) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("garbled sysvar {id} from upstream ({e}); using the VM default");
            None
        }
    }
}

/// Sysvars taken from the cluster when the source has them (`engine::CLUSTER_SYSVARS`).
fn is_cluster_sysvar(k: &Address) -> bool {
    *k == sysvar::rent::id() || *k == sysvar::clock::id() || *k == sysvar::epoch_schedule::id()
}

/// Writes the request's accounts into the VM, simulates, and reads post-state.
/// `loaded` remembers program/programdata accounts already set in this VM: re-setting an
/// unchanged one would only re-run ELF verification. One whose fingerprint changed (an
/// upgrade) is set again, together with the program account that points at it.
pub fn run_in(svm: &mut LiteSVM, loaded: &mut Loaded, input: SimInput) -> Result<SimOutcome, SimError> {
    let current: Clock = svm.get_sysvar();
    let clock = match cluster_sysvar::<Clock>(&input.accounts, sysvar::clock::id()) {
        Some(cluster) => cluster_clock(&current, &cluster, input.slot),
        // No cluster Clock: keep the VM clock monotonic (programs set at slot S are only
        // visible from S) and use wall-clock time.
        None => Clock { slot: current.slot.max(input.slot), unix_timestamp: now_unix(), ..current },
    };
    svm.set_sysvar(&clock);
    if let Some(schedule) = cluster_sysvar::<solana_epoch_schedule::EpochSchedule>(&input.accounts, sysvar::epoch_schedule::id()) {
        svm.set_sysvar(&schedule);
    }
    // The cluster's Rent replaces the VM's default (devnet's rent differs from LiteSVM's).
    if let Some(rent) = cluster_sysvar::<solana_rent::Rent>(&input.accounts, sysvar::rent::id()) {
        svm.set_sysvar(&rent);
    }

    // ProgramData set again in this request (vm_order puts it before its program).
    let mut reset: HashSet<Address> = HashSet::new();
    for (k, acct) in vm_order(input.accounts) {
        match acct {
            // Cluster sysvars were applied above (or the VM default kept); other sysvars,
            // builtins and precompiles come from the VM itself.
            _ if is_cluster_sysvar(&k) => {}
            Some(a) if a.owner == native_loader::id() || a.owner == sysvar::id() => {}
            Some(a) if a.executable || is_programdata(&a) => {
                let fp = Fingerprint::of(&a.data);
                let stale_program = programdata_of(&a).is_some_and(|pd| reset.contains(&pd));
                if !stale_program && loaded.get(&k) == Some(&fp) {
                    continue;
                }
                svm.set_account(k, a).map_err(|_| SimError::UnsupportedProgram(k))?;
                loaded.insert(k, fp);
                reset.insert(k);
            }
            Some(a) => {
                loaded.remove(&k);
                svm.set_account(k, a).map_err(|_| SimError::UnsupportedProgram(k))?;
            }
            None => {
                loaded.remove(&k);
                // Absent upstream. Never wipe a program the VM ships with.
                if svm.get_account(&k).is_some_and(|a| a.executable) {
                    continue;
                }
                let _ = svm.set_account(k, Account::default());
            }
        }
    }

    let blockhash = svm.latest_blockhash();
    Ok(match svm.simulate_transaction(input.tx) {
        Ok(info) => SimOutcome {
            err: None,
            logs: info.meta.logs,
            units: info.meta.compute_units_consumed,
            fee: info.meta.fee,
            inner: info.meta.inner_instructions,
            return_data: info.meta.return_data,
            post: info.post_accounts.into_iter().map(|(k, a)| (k, Account::from(a))).collect(),
            blockhash,
            clock,
        },
        Err(failed) => SimOutcome {
            err: Some(failed.err),
            logs: failed.meta.logs,
            units: failed.meta.compute_units_consumed,
            fee: failed.meta.fee,
            inner: failed.meta.inner_instructions,
            return_data: failed.meta.return_data,
            post: HashMap::new(),
            blockhash,
            clock,
        },
    })
}

struct Job {
    input: SimInput,
    reply: oneshot::Sender<Result<SimOutcome, SimError>>,
}

/// One job on one worker. A panic inside the simulation is caught: the worker's VM
/// is rebuilt (its state is unknown) and the caller gets `SimError::Panicked`.
fn step(
    svm: &mut LiteSVM,
    loaded: &mut Loaded,
    runs: &mut u32,
    recycle_after: u32,
    input: SimInput,
    f: impl FnOnce(&mut LiteSVM, &mut Loaded, SimInput) -> Result<SimOutcome, SimError>,
) -> Result<SimOutcome, SimError> {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(svm, loaded, input)));
    *runs += 1;
    if res.is_err() || *runs >= recycle_after {
        (*svm, *loaded, *runs) = (new_vm(), Loaded::new(), 0);
    }
    res.unwrap_or(Err(SimError::Panicked))
}

/// N OS threads, each owning one LiteSVM (no Send/Sync requirement on LiteSVM).
pub struct Pool {
    jobs: crossbeam_channel::Sender<Job>,
}

impl Pool {
    pub fn new(size: usize, recycle_after: u32) -> Pool {
        let (tx, rx) = crossbeam_channel::unbounded::<Job>();
        for i in 0..size.max(1) {
            let rx = rx.clone();
            std::thread::Builder::new()
                .name(format!("aval-vm-{i}"))
                .spawn(move || {
                    let (mut svm, mut loaded, mut runs) = (new_vm(), Loaded::new(), 0u32);
                    for job in rx {
                        let _ = job.reply.send(step(&mut svm, &mut loaded, &mut runs, recycle_after, job.input, run_in));
                    }
                })
                .expect("spawn vm worker");
        }
        Pool { jobs: tx }
    }

    pub async fn run(&self, input: SimInput) -> Result<SimOutcome, SimError> {
        let (reply, rx) = oneshot::channel();
        self.jobs.send(Job { input, reply }).map_err(|_| SimError::PoolDown)?;
        rx.await.map_err(|_| SimError::PoolDown)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_hash::Hash;
    use solana_message::{Message, VersionedMessage};
    use solana_sdk_ids::system_program;

    fn key(n: u8) -> Address { Address::from([n; 32]) }
    fn wallet(l: u64) -> Account { Account { lamports: l, owner: system_program::id(), ..Account::default() } }
    fn transfer(from: Address, to: Address, l: u64) -> VersionedTransaction {
        let ix = solana_system_interface::instruction::transfer(&from, &to, l);
        let msg = Message::new_with_blockhash(&[ix], Some(&from), &Hash::new_from_array([7; 32]));
        VersionedTransaction { signatures: vec![Default::default()], message: VersionedMessage::Legacy(msg) }
    }
    fn input(tx: VersionedTransaction, accounts: Vec<(Address, Option<Account>)>) -> SimInput {
        SimInput { tx, accounts, slot: 1000 }
    }

    #[test]
    fn transfer_moves_lamports_and_charges_fee() {
        let (mut svm, mut loaded) = (new_vm(), HashMap::new());
        let out = run_in(&mut svm, &mut loaded, input(transfer(key(1), key(2), 1_000_000),
            vec![(key(1), Some(wallet(10_000_000_000))), (key(2), None)])).unwrap();
        assert!(out.err.is_none(), "{:?} {:?}", out.err, out.logs);
        assert_eq!(out.post[&key(2)].lamports, 1_000_000);
        assert_eq!(out.post[&key(1)].lamports, 10_000_000_000 - 1_000_000 - out.fee);
    }

    #[test]
    fn insufficient_funds_is_a_tx_error_not_a_crash() {
        let (mut svm, mut loaded) = (new_vm(), HashMap::new());
        let out = run_in(&mut svm, &mut loaded, input(transfer(key(1), key(2), 10_000_000_000_000),
            vec![(key(1), Some(wallet(10_000_000))), (key(2), None)])).unwrap();
        assert!(out.err.is_some());
        assert!(out.post.is_empty());
    }

    #[test]
    fn no_state_bleeds_between_requests() {
        let (mut svm, mut loaded) = (new_vm(), HashMap::new());
        // A: key(3) is funded inside A's input
        let a = run_in(&mut svm, &mut loaded, input(transfer(key(1), key(3), 1_000_000),
            vec![(key(1), Some(wallet(10_000_000_000))), (key(3), Some(wallet(5_000_000_000)))])).unwrap();
        assert!(a.err.is_none(), "{:?}", a.logs);
        // B: key(3) is absent upstream now -> must not still have A's 5 SOL
        let out = run_in(&mut svm, &mut loaded, input(transfer(key(3), key(4), 1_000_000),
            vec![(key(3), None), (key(4), None)])).unwrap();
        assert!(out.err.is_some(), "key(3) should have no funds in B");
    }

    #[test]
    fn absent_upstream_does_not_wipe_builtin_programs() {
        let (mut svm, mut loaded) = (new_vm(), HashMap::new());
        // Precondition: the guard is only exercised if the VM actually stores the system program.
        assert!(svm.get_account(&system_program::id()).is_some_and(|a| a.executable),
            "LiteSVM does not store an executable system program account; the guard is untestable");
        // System program reported absent (e.g. a fixture that does not list programs)
        let out = run_in(&mut svm, &mut loaded, input(transfer(key(1), key(2), 1_000_000),
            vec![(key(1), Some(wallet(10_000_000_000))), (key(2), None), (system_program::id(), None)])).unwrap();
        assert!(out.err.is_none(), "{:?}", out.logs);
        assert!(svm.get_account(&system_program::id()).is_some_and(|a| a.executable), "builtin was wiped");
    }

    #[test]
    fn panic_is_contained_and_vm_rebuilt() {
        let (mut svm, mut loaded, mut runs) = (new_vm(), HashMap::new(), 0u32);
        loaded.insert(key(9), Fingerprint::of(&[1]));
        let i = input(transfer(key(1), key(2), 1), vec![]);
        let r = step(&mut svm, &mut loaded, &mut runs, 100, i, |_, _, _| panic!("boom"));
        assert!(matches!(r, Err(SimError::Panicked)));
        assert!(loaded.is_empty(), "a rebuilt VM has no programs loaded");
        // The same worker state keeps serving.
        let i = input(transfer(key(1), key(2), 1_000_000),
            vec![(key(1), Some(wallet(10_000_000_000))), (key(2), None)]);
        let r = step(&mut svm, &mut loaded, &mut runs, 100, i, run_in).unwrap();
        assert!(r.err.is_none(), "{:?}", r.logs);
    }

    #[test]
    fn recycling_clears_the_loaded_map() {
        let (mut svm, mut loaded, mut runs) = (new_vm(), HashMap::new(), 0u32);
        let ok = |svm: &mut LiteSVM, loaded: &mut Loaded, _: SimInput| {
            loaded.insert(key(9), Fingerprint::of(&[1]));
            run_in(svm, loaded, input(transfer(key(1), key(2), 1_000_000),
                vec![(key(1), Some(wallet(10_000_000_000))), (key(2), None)]))
        };
        step(&mut svm, &mut loaded, &mut runs, 2, input(transfer(key(1), key(2), 1), vec![]), ok).unwrap();
        assert_eq!(loaded.len(), 1);
        step(&mut svm, &mut loaded, &mut runs, 2, input(transfer(key(1), key(2), 1), vec![]), ok).unwrap();
        assert!(loaded.is_empty());
        assert_eq!(runs, 0);
    }

    #[test]
    fn vm_clock_never_moves_backwards() {
        let (mut svm, mut loaded) = (new_vm(), HashMap::new());
        let mk = |slot| SimInput { slot, ..input(transfer(key(1), key(2), 1_000_000),
            vec![(key(1), Some(wallet(10_000_000_000))), (key(2), None)]) };
        // LiteSVM starts at a large slot, so go above it to make the request slot authoritative.
        let base = svm.get_sysvar::<Clock>().slot + 1000;
        run_in(&mut svm, &mut loaded, mk(base)).unwrap();
        assert_eq!(svm.get_sysvar::<Clock>().slot, base);
        let out = run_in(&mut svm, &mut loaded, mk(base - 1)).unwrap();
        assert!(out.err.is_none(), "{:?}", out.logs);
        assert_eq!(svm.get_sysvar::<Clock>().slot, base, "clock moved backwards");
    }

    fn ed25519_tx(payer: Address, corrupt: bool) -> VersionedTransaction {
        use ed25519_dalek::Signer;
        let signer = ed25519_dalek::SigningKey::from_bytes(&[9u8; 32]);
        let message = b"aval precompile";
        let signature = signer.sign(message);
        let mut ix = solana_ed25519_program::new_ed25519_instruction_with_signature(
            message, &signature.to_bytes(), &signer.verifying_key().to_bytes());
        if corrupt {
            ix.data[solana_ed25519_program::DATA_START + 32] ^= 0xff;
        }
        let msg = Message::new_with_blockhash(&[ix], Some(&payer), &Hash::new_from_array([7; 32]));
        VersionedTransaction { signatures: vec![Default::default()], message: VersionedMessage::Legacy(msg) }
    }

    #[test]
    fn ed25519_precompile_verifies_signatures() {
        let (mut svm, mut loaded) = (new_vm(), HashMap::new());
        let ok = run_in(&mut svm, &mut loaded, input(ed25519_tx(key(1), false),
            vec![(key(1), Some(wallet(10_000_000_000)))])).unwrap();
        assert!(ok.err.is_none(), "{:?} {:?}", ok.err, ok.logs);
        let bad = run_in(&mut svm, &mut loaded, input(ed25519_tx(key(1), true),
            vec![(key(1), Some(wallet(10_000_000_000)))])).unwrap();
        // PrecompileError::InvalidSignature surfaces as Custom(2).
        assert_eq!(bad.err, Some(TransactionError::InstructionError(0, solana_instruction::error::InstructionError::Custom(2))));
    }

    fn sysvar_account<T: serde::Serialize>(v: &T) -> Option<Account> {
        Some(Account { lamports: 1, data: bincode::serialize(v).unwrap(), owner: sysvar::id(), ..Account::default() })
    }

    #[test]
    fn cluster_clock_keeps_cluster_fields_and_takes_the_highest_slot() {
        let vm = Clock { slot: 50, unix_timestamp: 1, epoch: 1, ..Clock::default() };
        let cluster = Clock { slot: 40, epoch_start_timestamp: 7, epoch: 9, leader_schedule_epoch: 10, unix_timestamp: 1_700_000_000 };
        let c = cluster_clock(&vm, &cluster, 45);
        assert_eq!(c, Clock { slot: 50, ..cluster.clone() });
        assert_eq!(cluster_clock(&vm, &cluster, 60).slot, 60);
        assert_eq!(cluster_clock(&vm, &Clock { slot: 70, ..cluster.clone() }, 60).slot, 70);
    }

    #[test]
    fn vm_uses_the_clusters_clock_and_epoch_schedule() {
        let (mut svm, mut loaded) = (new_vm(), HashMap::new());
        let base = svm.get_sysvar::<Clock>().slot + 1000;
        let cluster = Clock { slot: base, epoch_start_timestamp: 1_699_000_000, epoch: 777, leader_schedule_epoch: 778, unix_timestamp: 1_700_000_000 };
        let schedule = solana_epoch_schedule::EpochSchedule::custom(8192, 8192, false);
        let out = run_in(&mut svm, &mut loaded, SimInput { slot: base - 5, ..input(transfer(key(1), key(2), 1_000_000), vec![
            (key(1), Some(wallet(10_000_000_000))), (key(2), None),
            (sysvar::clock::id(), sysvar_account(&cluster)),
            (sysvar::epoch_schedule::id(), sysvar_account(&schedule)),
        ]) }).unwrap();
        assert!(out.err.is_none(), "{:?}", out.logs);
        assert_eq!(svm.get_sysvar::<Clock>(), cluster);
        assert_eq!(out.clock, cluster);
        assert_eq!(svm.get_sysvar::<solana_epoch_schedule::EpochSchedule>(), schedule);
    }

    #[test]
    fn absent_or_garbled_clock_falls_back_to_wall_clock() {
        let (mut svm, mut loaded) = (new_vm(), HashMap::new());
        let garbled = Some(Account { lamports: 1, data: vec![1, 2, 3], owner: sysvar::id(), ..Account::default() });
        for clock in [None, garbled] {
            let out = run_in(&mut svm, &mut loaded, input(transfer(key(1), key(2), 1_000_000), vec![
                (key(1), Some(wallet(10_000_000_000))), (key(2), None), (sysvar::clock::id(), clock),
                (sysvar::epoch_schedule::id(), None), (sysvar::rent::id(), None),
            ])).unwrap();
            assert!(out.err.is_none(), "{:?}", out.logs);
            let c = svm.get_sysvar::<Clock>();
            assert!((c.unix_timestamp - now_unix()).abs() < 5, "wall-clock fallback");
        }
    }

    #[test]
    fn fingerprint_is_full_data_up_to_64_bytes_then_len_and_prefix() {
        assert_eq!(Fingerprint::of(&[1, 2, 3]), Fingerprint::Full(vec![1, 2, 3]));
        let mut big = vec![5u8; 100];
        let f = Fingerprint::of(&big);
        assert_eq!(f, Fingerprint::Prefix(100, [5u8; 64]));
        big[99] = 0; // beyond the prefix: not seen (an ELF change always comes with a new deploy slot)
        assert_eq!(Fingerprint::of(&big), f);
        big[4] = 0; // ProgramData header (deploy slot) lives in the prefix
        assert_ne!(Fingerprint::of(&big), f);
    }

    /// An upgradeable program at `id` whose ProgramData (at `pd`) holds `elf`, deployed at `slot`.
    fn upgradeable(id: Address, pd: Address, elf: &[u8], slot: u64) -> Vec<(Address, Option<Account>)> {
        let mut pd_data = vec![3, 0, 0, 0];
        pd_data.extend_from_slice(&slot.to_le_bytes());
        pd_data.push(1);
        pd_data.extend_from_slice(key(42).as_ref());
        pd_data.extend_from_slice(elf);
        let mut prog_data = vec![2, 0, 0, 0];
        prog_data.extend_from_slice(pd.as_ref());
        let owner = solana_sdk_ids::bpf_loader_upgradeable::id();
        vec![
            (pd, Some(Account { lamports: 1_000_000_000, data: pd_data, owner, executable: false, rent_epoch: 0 })),
            (id, Some(Account { lamports: 1_000_000_000, data: prog_data, owner, executable: true, rent_epoch: 0 })),
        ]
    }

    fn call(program: Address, payer: Address) -> VersionedTransaction {
        let ix = solana_instruction::Instruction { program_id: program, accounts: vec![], data: b"hi".to_vec() };
        let msg = Message::new_with_blockhash(&[ix], Some(&payer), &Hash::new_from_array([7; 32]));
        VersionedTransaction { signatures: vec![Default::default()], message: VersionedMessage::Legacy(msg) }
    }

    #[test]
    fn upgraded_programdata_is_reloaded_in_the_same_vm() {
        let (mut svm, mut loaded) = (new_vm(), HashMap::new());
        let elf_of = |svm: &LiteSVM, id: &str| svm.get_account(&id.parse().unwrap()).unwrap().data;
        let memo = elf_of(&svm, "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
        let ata = elf_of(&svm, "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
        let (prog, pd) = (key(50), key(51));
        let slot = svm.get_sysvar::<Clock>().slot + 10;
        let run = |svm: &mut LiteSVM, loaded: &mut Loaded, elf: &[u8], deployed: u64| {
            let mut accounts = vec![(key(1), Some(wallet(10_000_000_000)))];
            accounts.extend(upgradeable(prog, pd, elf, deployed));
            run_in(svm, loaded, SimInput { tx: call(prog, key(1)), accounts, slot }).unwrap()
        };
        let v1 = run(&mut svm, &mut loaded, &memo, 1);
        assert!(v1.err.is_none(), "memo accepts utf-8: {:?}", v1.logs);
        assert_eq!(loaded.len(), 2);
        // Same bytes again: nothing is re-set (the VM's copy is left alone).
        let marker = Account { lamports: 7, ..svm.get_account(&pd).unwrap() };
        svm.set_account(pd, marker).unwrap();
        run(&mut svm, &mut loaded, &memo, 1);
        assert_eq!(svm.get_account(&pd).unwrap().lamports, 7, "unchanged programdata was re-set");
        // Upgrade: new deploy slot and a different ELF -> both accounts are re-set and the new code runs.
        let v2 = run(&mut svm, &mut loaded, &ata, 2);
        assert_eq!(svm.get_account(&pd).unwrap().data[4..12], 2u64.to_le_bytes());
        assert_ne!(v2.logs, v1.logs, "the upgraded program must run: {:?}", v2.logs);
        assert!(v2.err.is_some(), "the ATA program rejects a memo instruction: {:?}", v2.logs);
    }

    #[tokio::test]
    async fn pool_runs_on_worker_threads() {
        let pool = Pool::new(2, 100);
        let out = pool.run(input(transfer(key(1), key(2), 1_000_000),
            vec![(key(1), Some(wallet(10_000_000_000))), (key(2), None)])).await.unwrap();
        assert!(out.err.is_none());
    }
}
