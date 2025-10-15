use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::program_error::ProgramError;
use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, pubkey::Pubkey,
};

mod token;

#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub enum Ix {
    /// Accounts:
    /// 0. [signer,writable] payer
    /// 2. []                system_program
    CreateAndInitMint {
        mint_authority: Pubkey,
        decimals: u8,
        bump: u8,
    },

    /// Accounts:
    /// 0. [signer] payer
    /// 1. []       owner
    /// 2. [writable] ata
    /// 3. []       token_mint
    /// 4. []       token_program
    /// 5. []       system_program
    CreateAtaFor,

    /// Burn `amount_ui` whole tokens (UI units, not base units)
    /// Accounts:
    /// 0. []       mint
    /// 1. [signer] owner (authority of token account)
    /// 2. [writable] token_account (owner's ATA)
    /// 3. []       spl_token program (Tokenkeg…)
    BurnUserTokens { amount_ui: u64 },
}

impl Ix {
    pub fn unpack(input: &[u8]) -> Result<Self, ProgramError> {
        Self::try_from_slice(input).map_err(|_| ProgramError::InvalidInstructionData)
    }

    pub fn pack(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("Borsh serialization cannot fail")
    }
}

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    data: &[u8],
) -> ProgramResult {
    let ix = Ix::try_from_slice(data).map_err(|_| ProgramError::InvalidInstructionData)?;
    match ix {
        Ix::CreateAndInitMint {
            mint_authority,
            decimals,
            bump,
        } => {
            let seeds: &[&[u8]] = &[b"MINT", &[bump]];
            token::create_and_init_mint(program_id, accounts, &mint_authority, seeds, decimals)
        }
        Ix::CreateAtaFor => token::create_ata_for(accounts),
        Ix::BurnUserTokens { amount_ui } => token::burn_user_tokens(accounts, amount_ui),
    }
}

entrypoint!(process_instruction);
