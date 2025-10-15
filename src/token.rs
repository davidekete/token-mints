use solana_program::account_info::{next_account_info, AccountInfo};
use solana_program::entrypoint::ProgramResult;
use solana_program::program::{invoke, invoke_signed};
use solana_program::program_error::ProgramError;
use solana_program::program_pack::Pack;
use solana_program::pubkey::Pubkey;
use solana_program::rent::Rent;
use solana_program::sysvar::Sysvar;

use spl_associated_token_account as ata;
use spl_token::id as spl_token_id;
use spl_token::instruction as token_instruction;
use spl_token::state::{Account as SplAccount, Mint as SplMint};

pub fn create_and_init_mint(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    mint_authority: &Pubkey,
    mint_seeds: &[&[u8]],
    token_decimals: u8,
) -> ProgramResult {
    let acc_iter = &mut accounts.iter();

    //payer (signer), mint (writable), system program
    let payer = next_account_info(acc_iter)?;
    let token_mint = next_account_info(acc_iter)?;
    let system_program = next_account_info(acc_iter)?;
    let token_program = next_account_info(acc_iter)?;

    if !payer.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    if !token_mint.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }

    // Ensure the passed mint is exactly the PDA we expect for these seeds.
    let expected = Pubkey::create_program_address(mint_seeds, program_id)
        .map_err(|_| ProgramError::InvalidSeeds)?;
    if *token_mint.key != expected {
        return Err(ProgramError::InvalidSeeds);
    }

    let space = SplMint::LEN as u64;
    let lamports = Rent::get()?.minimum_balance(space as usize);

    invoke_signed(
        &solana_system_interface::instruction::create_account(
            &payer.key,
            &token_mint.key,
            lamports,
            space,
            &spl_token_id(),
        ),
        &[
            token_program.clone(),
            payer.clone(),
            token_mint.clone(),
            system_program.clone(),
        ],
        &[mint_seeds],
    )?;

    let initialize_ix = token_instruction::initialize_mint2(
        &spl_token_id(),
        token_mint.key,
        mint_authority,
        None,
        token_decimals,
    )?;

    invoke(&initialize_ix, &[token_mint.clone()])?;

    Ok(())
}

pub fn create_ata_for(accounts: &[AccountInfo]) -> ProgramResult {
    let acc_iter = &mut accounts.iter();

    let payer = next_account_info(acc_iter)?; //pays the fees for account creation
    let owner = next_account_info(acc_iter)?; //Owner of the ATA
    let ata_acc = next_account_info(acc_iter)?;
    let token_mint = next_account_info(acc_iter)?;
    let token_program = next_account_info(acc_iter)?;
    let system_program = next_account_info(acc_iter)?;

    // Derive the expected ATA address and compare
    let expected_ata = ata::get_associated_token_address_with_program_id(
        owner.key,
        token_mint.key,
        &spl_token_id(),
    );

    // Sanity check: is the passed ATA the one we expect?
    if ata_acc.key != &expected_ata {
        return Err(ProgramError::InvalidArgument);
    }

    // Payer must sign
    if !payer.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Create the ATA (idempotent)
    let ata_instruction = ata::instruction::create_associated_token_account_idempotent(
        payer.key,
        owner.key,
        token_mint.key,
        &spl_token_id(),
    );

    // Invoke the ATA creation instruction
    invoke(
        &ata_instruction,
        &[
            payer.clone(),
            ata_acc.clone(),
            owner.clone(),
            token_mint.clone(),
            system_program.clone(),
            token_program.clone(),
        ],
    )?;

    Ok(())
}

pub fn burn_user_tokens(accounts: &[AccountInfo], amount_ui: u64) -> ProgramResult {
    let acc_iter = &mut accounts.iter();

    // 0 mint, 1 owner(signer), 2 token_account(ATA), 3 token_program
    let mint_account = next_account_info(acc_iter)?;
    let owner_account = next_account_info(acc_iter)?; // authority, must sign
    let ata_token_acc = next_account_info(acc_iter)?;

    if !owner_account.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Sanity: token account belongs to owner and matches mint
    let token_account = SplAccount::unpack(&ata_token_acc.try_borrow_data()?)?;
    if token_account.mint != *mint_account.key || token_account.owner != *owner_account.key {
        return Err(ProgramError::InvalidAccountData);
    }

    // Derive base units using mint decimals
    let mint = SplMint::unpack(&mint_account.try_borrow_data()?)?;
    let decimals = mint.decimals;
    let amount_base = amount_ui
        .checked_mul(10u64.pow(decimals as u32))
        .ok_or(ProgramError::InvalidArgument)?;

    // Burn (checked) — accounts: [token_account, mint, authority]
    let burn_ix = token_instruction::burn_checked(
        &spl_token_id(),
        ata_token_acc.key,
        mint_account.key,
        owner_account.key,
        &[],
        amount_base,
        decimals,
    )?;

    invoke(
        &burn_ix,
        &[
            ata_token_acc.clone(),
            mint_account.clone(),
            owner_account.clone(),
        ],
    )?;

    Ok(())
}
