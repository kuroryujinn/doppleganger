use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

declare_id!("11111111111111111111111111111112");

#[program]
pub mod dg_escrow_v2 {
    use super::*;

    pub fn migrate_escrow(
        ctx: Context<MigrateEscrow>,
        deadline: i64,
        arbitrator: Pubkey,
        fail_after_write: bool,
    ) -> Result<()> {
        require!(
            deadline > Clock::get()?.unix_timestamp,
            EscrowError::InvalidDeadline
        );
        require_keys_neq!(
            arbitrator,
            Pubkey::default(),
            EscrowError::InvalidArbitrator
        );
        let initializer = ctx.accounts.escrow.initializer;
        let beneficiary = ctx.accounts.escrow.beneficiary;
        let amount = ctx.accounts.escrow.amount;
        let status = ctx.accounts.escrow.status;
        // Resize must retain the funds owed to the beneficiary as well as rent.
        let required = Rent::get()?
            .minimum_balance(8 + EscrowV2::INIT_SPACE)
            .checked_add(if status == EscrowStatus::Funded {
                amount
            } else {
                0
            })
            .ok_or(EscrowError::Overflow)?;
        let shortfall = required.saturating_sub(ctx.accounts.escrow.to_account_info().lamports());
        if shortfall > 0 {
            system_program::transfer(
                CpiContext::new(
                    System::id(),
                    Transfer {
                        from: ctx.accounts.initializer.to_account_info(),
                        to: ctx.accounts.escrow.to_account_info(),
                    },
                ),
                shortfall,
            )?;
        }
        #[cfg(feature = "r9-broken-amount")]
        let amount = amount
            .checked_sub(100_000)
            .ok_or(EscrowError::InvalidAmount)?;
        ctx.accounts.escrow.migrate(EscrowV2 {
            initializer,
            beneficiary,
            amount,
            deadline,
            arbitrator,
            status,
        })?;
        if fail_after_write {
            // Controlled fixture path: a real byte write precedes the rejection.
            ctx.accounts
                .escrow
                .to_account_info()
                .try_borrow_mut_data()?[72] ^= 1;
            msg!("attempted escrow write, resize and rent top-up before rejection");
            return err!(EscrowError::DeliberateFailure);
        }
        Ok(())
    }

    pub fn release(ctx: Context<Release>) -> Result<()> {
        require!(
            ctx.accounts.escrow.status == EscrowStatus::Funded,
            EscrowError::WrongState
        );
        require!(
            Clock::get()?.unix_timestamp <= ctx.accounts.escrow.deadline,
            EscrowError::Expired
        );
        let amount = ctx.accounts.escrow.amount;
        let source = ctx.accounts.escrow.to_account_info();
        let destination = ctx.accounts.beneficiary.to_account_info();
        let remaining = source
            .lamports()
            .checked_sub(amount)
            .ok_or(EscrowError::InvalidAmount)?;
        require!(
            remaining >= Rent::get()?.minimum_balance(source.data_len()),
            EscrowError::InvalidAmount
        );
        let received = destination
            .lamports()
            .checked_add(amount)
            .ok_or(EscrowError::Overflow)?;
        **source.try_borrow_mut_lamports()? = remaining;
        **destination.try_borrow_mut_lamports()? = received;
        ctx.accounts.escrow.status = EscrowStatus::Released;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct MigrateEscrow<'info> {
    #[account(mut, constraint = escrow.initializer == initializer.key() @ EscrowError::Unauthorized,
        seeds = [b"escrow", escrow.initializer.as_ref(), escrow.beneficiary.as_ref()], bump,
        realloc = 8 + EscrowV2::INIT_SPACE, realloc::payer = initializer, realloc::zero = false)]
    pub escrow: Migration<'info, EscrowV1, EscrowV2>,
    #[account(mut)]
    pub initializer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Release<'info> {
    #[account(mut, has_one = beneficiary,
        seeds = [b"escrow", escrow.initializer.as_ref(), beneficiary.key().as_ref()], bump)]
    pub escrow: Account<'info, EscrowV2>,
    #[account(mut)]
    pub beneficiary: Signer<'info>,
}

// The first four enum tags retain their V1 encoding; Disputed is appended in V2.
#[account]
#[derive(InitSpace)]
pub struct EscrowV1 {
    pub initializer: Pubkey,
    pub beneficiary: Pubkey,
    pub amount: u64,
    pub status: EscrowStatus,
}

#[account]
#[derive(InitSpace)]
pub struct EscrowV2 {
    pub initializer: Pubkey,
    pub beneficiary: Pubkey,
    pub amount: u64,
    pub deadline: i64,
    pub arbitrator: Pubkey,
    pub status: EscrowStatus,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum EscrowStatus {
    Created,
    Funded,
    Released,
    Refunded,
    Disputed,
}

#[error_code]
pub enum EscrowError {
    #[msg("Escrow amount is invalid")]
    InvalidAmount,
    #[msg("Escrow parties must be distinct")]
    SameParty,
    #[msg("Escrow is not in the required state")]
    WrongState,
    #[msg("Only the initializer can migrate")]
    Unauthorized,
    #[msg("Migration deadline must be in the future")]
    InvalidDeadline,
    #[msg("Arbitrator cannot be the zero public key")]
    InvalidArbitrator,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("Controlled migration failure")]
    DeliberateFailure,
    #[msg("Escrow deadline has elapsed")]
    Expired,
}
