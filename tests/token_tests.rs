use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::example_mocks::solana_sdk::{system_program, sysvar};
use solana_program_test::*;
use solana_program_test::{processor, ProgramTest};
use solana_sdk::{
    instruction::{AccountMeta, Instruction},
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::Transaction,
};

use solana_program::program_pack::Pack;
use spl_associated_token_account as ata;
use spl_token::{
    self, id as spl_token_id,
    state::{Account as SplAccount, Mint as SplMint},
};

use token_mints::Ix; // your crate name = package name; adjust if needed

fn program_test() -> (ProgramTest, Pubkey) {
    let program_id = Pubkey::new_unique();

    let mut program_test = ProgramTest::new(
        "tokens",
        program_id,
        processor!(token_mints::process_instruction),
    );

    // Register SPL Token program so CPI works
    program_test.add_program(
        "spl_token",
        spl_token::id(),
        processor!(spl_token::processor::Processor::process),
    );

    // Register ATA program so CPI works
    program_test.add_program(
        "spl_associated_token_account",
        ata::id(),
        processor!(ata::processor::process_instruction),
    );

    assert_eq!(
        spl_token::id().to_string(),
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
    );
    msg!("spl token id {}", spl_token::id().to_string());

    (program_test, program_id)
}

#[tokio::test]
async fn test_create_and_init_mint_then_create_ata() {
    let (mut pt, program_id) = program_test();
    let mut ctx = pt.start_with_context().await;

    let payer = &ctx.payer; // funded by program-test
    let recent_blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();

    // ---------- Create & init Mint ----------
    let (mint_pda, bump) = Pubkey::find_program_address(&[b"MINT"], &program_id);

    // Build instruction data
    let ix_data = Ix::CreateAndInitMint {
        mint_authority: payer.pubkey(),
        decimals: 6,
        bump,
    }
    .pack();

    // Accounts: payer, mint, system_program
    let create_mint_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new(mint_pda, false),
            AccountMeta::new_readonly(system_program::ID, false),
            AccountMeta::new_readonly(spl_token::id(), false),
        ],
        data: ix_data,
    };

    // Send tx
    let mut tx = Transaction::new_with_payer(&[create_mint_ix], Some(&payer.pubkey()));
    tx.sign(&[payer], recent_blockhash);
    ctx.banks_client.process_transaction(tx).await.unwrap();

    // Assert mint initialized
    let mint_acc = ctx
        .banks_client
        .get_account(mint_pda)
        .await
        .unwrap()
        .expect("mint exists");
    // Owned by SPL token program
    assert_eq!(mint_acc.owner, spl_token_id());
    let mint_state = SplMint::unpack(&mint_acc.data).unwrap();
    assert_eq!(mint_state.decimals, 6);
    assert_eq!(mint_state.mint_authority.unwrap(), payer.pubkey());
    assert!(mint_acc.lamports > 0); // rent-exempt

    // ---------- Create ATA (idempotent) ----------
    let owner = Keypair::new();
    let ata_addr = ata::get_associated_token_address_with_program_id(
        &owner.pubkey(),
        &mint_pda,
        &spl_token_id(),
    );

    // First call
    let ix_data = Ix::CreateAtaFor.pack();
    let create_ata_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(payer.pubkey(), true),                   // payer
            AccountMeta::new_readonly(owner.pubkey(), false),         // owner
            AccountMeta::new(ata_addr, false),                        // ata
            AccountMeta::new_readonly(mint_pda, false),               // mint
            AccountMeta::new_readonly(spl_token::id(), false),        // token program
            AccountMeta::new_readonly(system_program::ID, false),     // system program
            AccountMeta::new_readonly(sysvar::rent::id(), false),        // rent
            AccountMeta::new_readonly(ata::id(), false),
        ],
        data: ix_data,
    };
    let recent_blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let mut tx = Transaction::new_with_payer(&[create_ata_ix.clone()], Some(&payer.pubkey()));
    tx.sign(&[payer], recent_blockhash);
    ctx.banks_client.process_transaction(tx).await.unwrap();

    // Second call (should be idempotent & succeed)
    let recent_blockhash = ctx.banks_client.get_latest_blockhash().await.unwrap();
    let mut tx2 = Transaction::new_with_payer(&[create_ata_ix], Some(&payer.pubkey()));
    tx2.sign(&[payer], recent_blockhash);
    ctx.banks_client.process_transaction(tx2).await.unwrap();

    // Assert ATA state
    let ata_acc = ctx
        .banks_client
        .get_account(ata_addr)
        .await
        .unwrap()
        .expect("ata exists");
    assert_eq!(ata_acc.owner, spl_token_id()); // token program owns token accounts

    let ata_state = SplAccount::unpack(&ata_acc.data).unwrap();
    assert_eq!(ata_state.mint, mint_pda);
    assert_eq!(ata_state.owner, owner.pubkey());
    assert_eq!(ata_state.amount, 0); // empty by default

    // let decimals = 6u8;
    // let mint_amount_ui: u64 = 5;                   // 5 tokens
    // let mint_amount_base = mint_amount_ui * 10u64.pow(decimals as u32);
    //
    // let mint_to_ix = spl_token::instruction::mint_to_checked(
    //     &spl_token::id(),
    //     &mint_pda,                 // mint
    //     &ata_addr,                 // destination ATA
    //     &payer.pubkey(),           // mint authority (we set payer as authority at init)
    //     &[],                       // multisig signers
    //     mint_amount_base,
    //     decimals,
    // ).unwrap();
    //
    // let recent = ctx.banks_client.get_latest_blockhash().await.unwrap();
    // let mut tx = Transaction::new_with_payer(&[mint_to_ix], Some(&payer.pubkey()));
    // tx.sign(&[payer], recent);
    // ctx.banks_client.process_transaction(tx).await.unwrap();
    //
    // // Confirm minted
    // let ata_acc = ctx.banks_client.get_account(ata_addr).await.unwrap().expect("ata exists");
    // let ata_state_before = SplAccount::unpack_from_slice(&ata_acc.data).unwrap();
    // assert_eq!(ata_state_before.amount, mint_amount_base);
    //
    // // ---------- Burn via your program ----------
    // let burn_ui: u64 = 2; // burn 2 whole tokens
    // let burn_ui_data = Ix::BurnUserTokens { amount_ui: burn_ui }.pack();
    // let burn_ix = Instruction {
    //     program_id,
    //     accounts: vec![
    //         AccountMeta::new_readonly(mint_pda, false),            // mint
    //         AccountMeta::new(owner.pubkey(), true),                // owner signer!
    //         AccountMeta::new(ata_addr, false),                     // token account (ATA)
    //         AccountMeta::new_readonly(spl_token::id(), false),     // token program present for CPI
    //     ],
    //     data: burn_ui_data,
    // };
    //
    // let recent = ctx.banks_client.get_latest_blockhash().await.unwrap();
    // let mut tx = Transaction::new_with_payer(&[burn_ix], Some(&payer.pubkey()));
    // tx.sign(&[payer, &owner], recent); // payer pays fees; owner signs as burn authority
    // ctx.banks_client.process_transaction(tx).await.unwrap();
    //
    // // ---------- Assert burned ----------
    // let ata_acc = ctx.banks_client.get_account(ata_addr).await.unwrap().expect("ata exists");
    // let ata_state_after = SplAccount::unpack_from_slice(&ata_acc.data).unwrap();
    // let expected_after = mint_amount_base - burn_ui * 10u64.pow(decimals as u32);
    // assert_eq!(ata_state_after.amount, expected_after);
}

