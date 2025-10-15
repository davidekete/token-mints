// tests/token_tests.rs

use solana_program::program_pack::Pack;
use solana_program::sysvar;
use solana_program_test::{processor, ProgramTest, ProgramTestContext};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    system_program,
    transaction::Transaction,
};

use spl_associated_token_account as ata;
use spl_token::{self, state::{Account as SplAccount, Mint as SplMint}};

use token_mints::Ix; // your program’s instruction enum with .pack()

// ---------- Test harness ----------

fn program_test() -> (ProgramTest, Pubkey) {
    let program_id = Pubkey::new_unique();

    let mut pt = ProgramTest::new(
        "tokens",
        program_id,
        processor!(token_mints::process_instruction),
    );

    // CPI targets
    pt.add_program(
        "spl_token",
        spl_token::id(),
        processor!(spl_token::processor::Processor::process),
    );
    pt.add_program(
        "spl_associated_token_account",
        ata::id(),
        processor!(ata::processor::process_instruction),
    );

    (pt, program_id)
}

fn find_mint_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"MINT"], program_id)
}

// ---------- Instruction builders (use Ix::pack()) ----------

fn ix_create_and_init_mint(
    program_id: Pubkey,
    payer: Pubkey,
    mint_pda: Pubkey,
    decimals: u8,
    bump: u8,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer, true),                               // payer
            AccountMeta::new(mint_pda, false),                           // mint PDA
            AccountMeta::new_readonly(system_program::ID, false),        // system
            AccountMeta::new_readonly(spl_token::id(), false),           // token program (for CPI)
        ],
        data: Ix::CreateAndInitMint { mint_authority: payer, decimals, bump }.pack(),
    }
}

fn ix_create_ata_via_program(
    program_id: Pubkey,
    payer: Pubkey,
    owner: Pubkey,
    ata_addr: Pubkey,
    mint: Pubkey,
) -> Instruction {
    // Your on-chain `create_ata_for` expects:
    // payer, owner, ata, mint, token_program, system_program, rent, ata_program
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(owner, false),
            AccountMeta::new(ata_addr, false),
            AccountMeta::new_readonly(mint, false),
            AccountMeta::new_readonly(spl_token::id(), false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(sysvar::rent::id(), false),
            AccountMeta::new_readonly(ata::id(), false),
        ],
        data: Ix::CreateAtaFor.pack(),
    }
}

fn ix_mint_to_checked(
    mint: Pubkey,
    dest_ata: Pubkey,
    mint_authority: Pubkey,
    amount_base: u64,
    decimals: u8,
) -> Instruction {
    spl_token::instruction::mint_to_checked(
        &spl_token::id(),
        &mint,
        &dest_ata,
        &mint_authority,
        &[],
        amount_base,
        decimals,
    ).unwrap()
}

fn ix_burn_ui_via_program(
    program_id: Pubkey,
    mint: Pubkey,
    owner: Pubkey,
    owner_ata: Pubkey,
    amount_ui: u64,
) -> Instruction {
    Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(mint, false),                              // ⬅️ writable mint
            AccountMeta::new(owner, true),                               // owner signer
            AccountMeta::new(owner_ata, false),                          // token account (writable ATA)
            AccountMeta::new_readonly(spl_token::id(), false),           // token program
        ],
        data: Ix::BurnUserTokens { amount_ui }.pack(),
    }
}


// ---------- Small utils (keep borrows disjoint) ----------

async fn latest_blockhash(ctx: &mut ProgramTestContext) -> solana_sdk::hash::Hash {
    ctx.banks_client.get_latest_blockhash().await.unwrap()
}

async fn process(ctx: &mut ProgramTestContext, tx: Transaction) {
    ctx.banks_client.process_transaction(tx).await.unwrap();
}

async fn assert_mint_state(
    ctx: &mut ProgramTestContext,
    mint: Pubkey,
    exp_authority: Pubkey,
    exp_decimals: u8,
) {
    let acc = ctx.banks_client.get_account(mint).await.unwrap().expect("mint exists");
    assert_eq!(acc.owner, spl_token::id());
    let state = SplMint::unpack_from_slice(&acc.data).unwrap();
    assert_eq!(state.mint_authority.unwrap(), exp_authority);
    assert_eq!(state.decimals, exp_decimals);
}

async fn token_amount(ctx: &mut ProgramTestContext, token_acc: Pubkey) -> u64 {
    let acc = ctx.banks_client.get_account(token_acc).await.unwrap().expect("token acc exists");
    let state = SplAccount::unpack_from_slice(&acc.data).unwrap();
    state.amount
}

// ---------- TEST 1: init mint ----------

#[tokio::test]
async fn test_init_mint() {
    let (mut pt, program_id) = program_test();
    let mut ctx = pt.start_with_context().await;

    // Local copy of payer keypair to avoid borrowing ctx across awaits
    let payer = Keypair::from_bytes(&ctx.payer.to_bytes()).unwrap();

    let (mint_pda, bump) = find_mint_pda(&program_id);
    let decimals = 6u8;

    let ix = ix_create_and_init_mint(program_id, payer.pubkey(), mint_pda, decimals, bump);

    let bh = latest_blockhash(&mut ctx).await;
    let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
    tx.sign(&[&payer], bh);
    process(&mut ctx, tx).await;

    assert_mint_state(&mut ctx, mint_pda, payer.pubkey(), decimals).await;
}

// ---------- TEST 2: create ATA for mint ----------

#[tokio::test]
async fn test_create_ata_for_mint() {
    let (mut pt, program_id) = program_test();
    let mut ctx = pt.start_with_context().await;

    let payer = Keypair::from_bytes(&ctx.payer.to_bytes()).unwrap();

    // pre: mint exists
    let (mint_pda, bump) = find_mint_pda(&program_id);
    let decimals = 6u8;
    {
        let ix = ix_create_and_init_mint(program_id, payer.pubkey(), mint_pda, decimals, bump);
        let bh = latest_blockhash(&mut ctx).await;
        let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
        tx.sign(&[&payer], bh);
        process(&mut ctx, tx).await;
    }

    // create ATA
    let owner = Keypair::new();
    let ata_addr = ata::get_associated_token_address_with_program_id(&owner.pubkey(), &mint_pda, &spl_token::id());

    let ix = ix_create_ata_via_program(program_id, payer.pubkey(), owner.pubkey(), ata_addr, mint_pda);
    let bh = latest_blockhash(&mut ctx).await;
    let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
    tx.sign(&[&payer], bh);
    process(&mut ctx, tx).await;

    // assert ATA linkage
    let acc = ctx.banks_client.get_account(ata_addr).await.unwrap().expect("ata exists");
    assert_eq!(acc.owner, spl_token::id());
    let state = SplAccount::unpack_from_slice(&acc.data).unwrap();
    assert_eq!(state.mint, mint_pda);
    assert_eq!(state.owner, owner.pubkey());
    assert_eq!(state.amount, 0);
}

// ---------- TEST 3: burn tokens ----------

#[tokio::test]
async fn test_burn_tokens() {
    let (mut pt, program_id) = program_test();
    let mut ctx = pt.start_with_context().await;

    let payer = Keypair::from_bytes(&ctx.payer.to_bytes()).unwrap();

    // pre: mint
    let (mint_pda, bump) = find_mint_pda(&program_id);
    let decimals = 6u8;
    {
        let ix = ix_create_and_init_mint(program_id, payer.pubkey(), mint_pda, decimals, bump);
        let bh = latest_blockhash(&mut ctx).await;
        let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
        tx.sign(&[&payer], bh);
        process(&mut ctx, tx).await;
    }

    // pre: ATA for owner
    let owner = Keypair::new();
    let ata_addr = ata::get_associated_token_address_with_program_id(&owner.pubkey(), &mint_pda, &spl_token::id());
    {
        let ix = ix_create_ata_via_program(program_id, payer.pubkey(), owner.pubkey(), ata_addr, mint_pda);
        let bh = latest_blockhash(&mut ctx).await;
        let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
        tx.sign(&[&payer], bh);
        process(&mut ctx, tx).await;
    }

    // mint 5 tokens to owner's ATA
    let ui = 5u64;
    let base = ui * 10u64.pow(decimals as u32);
    {
        let ix = ix_mint_to_checked(mint_pda, ata_addr, payer.pubkey(), base, decimals);
        let bh = latest_blockhash(&mut ctx).await;
        let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
        tx.sign(&[&payer], bh);
        process(&mut ctx, tx).await;
    }
    assert_eq!(token_amount(&mut ctx, ata_addr).await, base);

    // burn 2 tokens via your program (owner must sign as authority)
    let burn_ui = 2u64;
    {
        let ix = ix_burn_ui_via_program(program_id, mint_pda, owner.pubkey(), ata_addr, burn_ui);
        let bh = latest_blockhash(&mut ctx).await;
        let mut tx = Transaction::new_with_payer(&[ix], Some(&payer.pubkey()));
        tx.sign(&[&payer, &owner], bh);
        process(&mut ctx, tx).await;
    }

    let expected = base - burn_ui * 10u64.pow(decimals as u32);
    assert_eq!(token_amount(&mut ctx, ata_addr).await, expected);
}
