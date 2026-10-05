//! Execute the actual prototype SBF binary. The immutable Devnet oracle address
//! has no test secret in source: VM crypto verification is disabled for this
//! fixture, while account signer flags and address constraints remain enforced.
//! Node client tests separately verify cryptographic transaction signatures.
use anchor_lang::{prelude::Pubkey, AccountDeserialize, InstructionData, ToAccountMetas};
use litesvm::LiteSVM;
use solana_account::Account;
use solana_address::Address;
use solana_clock::Clock;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_program_option::COption;
use solana_program_pack::Pack;
use solana_signer::Signer;
use solana_transaction::Transaction;
use spl_token_interface::state::{Account as TokenAccount, AccountState, Mint};
use truhabit_prototype::{accounts, instruction, Commitment, Terms, ID, MINT, ORACLE, RECIPIENT};

const NOW: i64 = 1_800_000_000;
const AMOUNT: u64 = 5_000_000;
fn addr(p: Pubkey) -> Address {
    Address::new_from_array(p.to_bytes())
}
fn pk(p: Address) -> Pubkey {
    Pubkey::new_from_array(p.to_bytes())
}
fn ix(data: impl InstructionData, metas: impl ToAccountMetas) -> Instruction {
    Instruction {
        program_id: addr(ID),
        data: data.data(),
        accounts: metas
            .to_account_metas(None)
            .into_iter()
            .map(|a| AccountMeta {
                pubkey: addr(a.pubkey),
                is_signer: a.is_signer,
                is_writable: a.is_writable,
            })
            .collect(),
    }
}
struct Fixture {
    svm: LiteSVM,
    owner: Keypair,
    source: Pubkey,
    destination: Pubkey,
    recipient: Pubkey,
    commitment: Pubkey,
    vault: Pubkey,
}
impl Fixture {
    fn new() -> Self {
        let mut svm = LiteSVM::new().with_sigverify(false);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../prototype/target/deploy/truhabit_prototype.so");
        svm.add_program(
            addr(ID),
            &std::fs::read(path).expect("Build prototype SBF first"),
        )
        .unwrap();
        let owner = Keypair::new();
        svm.airdrop(&owner.pubkey(), 2_000_000_000).unwrap();
        svm.airdrop(&addr(ORACLE), 1_000_000).unwrap();
        let commitment =
            Pubkey::find_program_address(&[b"prototype", owner.pubkey().as_ref(), &[42; 16]], &ID)
                .0;
        let vault = Pubkey::find_program_address(&[b"vault", commitment.as_ref()], &ID).0;
        let mut f = Self {
            svm,
            owner,
            source: Pubkey::new_unique(),
            destination: Pubkey::new_unique(),
            recipient: Pubkey::new_unique(),
            commitment,
            vault,
        };
        f.mint(MINT);
        f.tokens(f.source, pk(f.owner.pubkey()), 100_000_000, MINT);
        f.tokens(f.destination, pk(f.owner.pubkey()), 0, MINT);
        f.tokens(f.recipient, RECIPIENT, 0, MINT);
        f.clock(NOW);
        f
    }
    fn clock(&mut self, at: i64) {
        let mut clock = self.svm.get_sysvar::<Clock>();
        clock.unix_timestamp = at;
        self.svm.set_sysvar(&clock);
        self.svm.expire_blockhash();
    }
    fn mint(&mut self, key: Pubkey) {
        let mut data = vec![0; Mint::LEN];
        Mint::pack(
            Mint {
                mint_authority: COption::Some(ORACLE),
                supply: 100_000_000,
                decimals: 6,
                is_initialized: true,
                freeze_authority: COption::None,
            },
            &mut data,
        )
        .unwrap();
        self.put(key, data);
    }
    fn tokens(&mut self, key: Pubkey, owner: Pubkey, amount: u64, mint: Pubkey) {
        let mut data = vec![0; TokenAccount::LEN];
        TokenAccount::pack(
            TokenAccount {
                mint,
                owner,
                amount,
                delegate: COption::None,
                state: AccountState::Initialized,
                is_native: COption::None,
                delegated_amount: 0,
                close_authority: COption::None,
            },
            &mut data,
        )
        .unwrap();
        self.put(key, data);
    }
    fn put(&mut self, key: Pubkey, data: Vec<u8>) {
        self.svm
            .set_account(
                addr(key),
                Account {
                    lamports: self.svm.minimum_balance_for_rent_exemption(data.len()),
                    data,
                    owner: addr(anchor_spl::token::ID),
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }
    fn terms(&self) -> Terms {
        Terms {
            id: [42; 16],
            amount: AMOUNT,
            starts: NOW + 10,
            ends: NOW + 100,
            upload: NOW + 200,
            refund: NOW + 300,
            terms_hash: [3; 32],
        }
    }
    fn deposit(&self, terms: Terms) -> Instruction {
        ix(
            instruction::Deposit { args: terms },
            accounts::Deposit {
                owner: pk(self.owner.pubkey()),
                oracle: ORACLE,
                mint: MINT,
                commitment: self.commitment,
                vault: self.vault,
                source: self.source,
                token_program: anchor_spl::token::ID,
                system_program: pk(solana_sdk_ids::system_program::ID),
            },
        )
    }
    fn settle(&self, action: u8, actor: Pubkey, destination: Pubkey) -> Instruction {
        ix(
            instruction::Settle { action },
            accounts::Settle {
                actor,
                mint: MINT,
                commitment: self.commitment,
                vault: self.vault,
                destination,
                token_program: anchor_spl::token::ID,
            },
        )
    }
    fn run(&mut self, instruction: Instruction) -> Result<(), String> {
        let block = self.svm.latest_blockhash();
        let message =
            Message::new_with_blockhash(&[instruction], Some(&self.owner.pubkey()), &block);
        let mut tx = Transaction::new_unsigned(message);
        tx.partial_sign(&[&self.owner], block);
        let result = self
            .svm
            .send_transaction(tx)
            .map(|_| ())
            .map_err(|e| format!("{:?}\n{}", e.err, e.meta.logs.join("\n")));
        self.svm.expire_blockhash();
        result
    }
    fn fund(&mut self) {
        self.run(self.deposit(self.terms())).unwrap();
        assert_eq!(self.balance(self.source), 100_000_000 - AMOUNT);
        assert_eq!(self.balance(self.vault), AMOUNT);
    }
    fn balance(&self, key: Pubkey) -> u64 {
        TokenAccount::unpack(&self.svm.get_account(&addr(key)).unwrap().data)
            .unwrap()
            .amount
    }
    fn state(&self) -> Commitment {
        Commitment::try_deserialize(
            &mut self
                .svm
                .get_account(&addr(self.commitment))
                .unwrap()
                .data
                .as_slice(),
        )
        .unwrap()
    }
}

#[test]
fn deposit_requires_fixed_oracle_signature_mint_and_valid_terms() {
    for attack in 0..5 {
        let mut f = Fixture::new();
        let mut terms = f.terms();
        if attack == 2 {
            terms.amount = 50_000_001;
        }
        if attack == 3 {
            terms.refund = terms.upload;
        }
        let mut instruction = f.deposit(terms);
        if attack == 0 {
            instruction.accounts[1].is_signer = false;
        }
        if attack == 1 {
            instruction.accounts[1].pubkey = f.owner.pubkey();
        }
        if attack == 4 {
            let mint = Pubkey::new_unique();
            f.mint(mint);
            instruction.accounts[2].pubkey = addr(mint);
        }
        assert!(f.run(instruction).is_err(), "attack {attack}");
        assert_eq!(f.balance(f.source), 100_000_000);
        assert!(f.svm.get_account(&addr(f.commitment)).is_none());
    }
}
#[test]
fn success_returns_exact_principal_and_tombstone_blocks_all_replays() {
    let mut f = Fixture::new();
    f.fund();
    f.clock(NOW + 10);
    f.tokens(f.vault, f.commitment, AMOUNT + 123, MINT);
    f.run(f.settle(0, ORACLE, f.destination)).unwrap();
    assert_eq!(f.balance(f.destination), AMOUNT);
    assert_eq!(f.balance(f.vault), 123);
    assert_eq!(f.state().status, 1);
    assert!(f.run(f.settle(0, ORACLE, f.destination)).is_err());
    assert!(f.run(f.deposit(f.terms())).is_err());
    assert_eq!(f.balance(f.destination), AMOUNT);
}
#[test]
fn failure_is_only_after_upload_deadline_and_to_fixed_recipient() {
    let mut f = Fixture::new();
    f.fund();
    f.clock(NOW + 200);
    assert!(f.run(f.settle(1, ORACLE, f.recipient)).is_err());
    f.clock(NOW + 201);
    assert!(f
        .run(f.settle(1, pk(f.owner.pubkey()), f.recipient))
        .is_err());
    assert!(f.run(f.settle(1, ORACLE, f.destination)).is_err());
    f.run(f.settle(1, ORACLE, f.recipient)).unwrap();
    assert_eq!(f.balance(f.recipient), AMOUNT);
    assert_eq!(f.balance(f.destination), 0);
    assert_eq!(f.state().status, 2);
}
#[test]
fn cancellation_is_owner_only_and_before_start() {
    let mut f = Fixture::new();
    f.fund();
    assert!(f.run(f.settle(2, ORACLE, f.destination)).is_err());
    f.clock(NOW + 9);
    f.run(f.settle(2, pk(f.owner.pubkey()), f.destination))
        .unwrap();
    assert_eq!(f.state().status, 3);
    assert_eq!(f.balance(f.destination), AMOUNT);
    let mut late = Fixture::new();
    late.fund();
    late.clock(NOW + 10);
    assert!(late
        .run(late.settle(2, pk(late.owner.pubkey()), late.destination))
        .is_err());
    assert_eq!(late.balance(late.vault), AMOUNT);
}
#[test]
fn hard_timeout_allows_owner_without_oracle_at_exact_boundary() {
    let mut f = Fixture::new();
    f.fund();
    f.clock(NOW + 299);
    assert!(f
        .run(f.settle(3, pk(f.owner.pubkey()), f.destination))
        .is_err());
    f.clock(NOW + 300);
    assert!(f.run(f.settle(0, ORACLE, f.destination)).is_err());
    assert!(f.run(f.settle(1, ORACLE, f.recipient)).is_err());
    f.run(f.settle(3, pk(f.owner.pubkey()), f.destination))
        .unwrap();
    assert_eq!(f.state().status, 4);
    assert_eq!(f.balance(f.destination), AMOUNT);
    assert!(f.run(f.settle(1, ORACLE, f.recipient)).is_err());
}
#[test]
fn success_requires_oracle_after_start_and_correct_destination() {
    let mut f = Fixture::new();
    f.fund();
    assert!(f.run(f.settle(0, ORACLE, f.destination)).is_err());
    f.clock(NOW + 10);
    assert!(f
        .run(f.settle(0, pk(f.owner.pubkey()), f.destination))
        .is_err());
    assert!(f.run(f.settle(0, ORACLE, f.recipient)).is_err());
    let mut missing = f.settle(0, ORACLE, f.destination);
    missing.accounts[0].is_signer = false;
    assert!(f.run(missing).is_err());
    assert_eq!(f.balance(f.vault), AMOUNT);
}
