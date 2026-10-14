use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

// Local LiteSVM fixture identity only; no deployment key is required.
declare_id!("11111111111111111111111111111112");

#[program]
pub mod dg_escrow_v1 {
    use super::*;

    pub fn initialize_escrow(ctx: Context<InitializeEscrow>, amount: u64) -> Result<()> {
        require!(amount > 0, EscrowError::InvalidAmount);
        require_keys_neq!(
            ctx.accounts.initializer.key(),
            ctx.accounts.beneficiary.key(),
            EscrowError::SameParty
        );
        ctx.accounts.escrow.set_inner(EscrowV1 {
            initializer: ctx.accounts.initializer.key(),
            beneficiary: ctx.accounts.beneficiary.key(),
            amount,
            status: EscrowStatus::Created,
        });
        Ok(())
    }

    pub fn fund(ctx: Context<Fund>) -> Result<()> {
        require!(
            ctx.accounts.escrow.status == EscrowStatus::Created,
            EscrowError::WrongState
        );
        system_program::transfer(
            CpiContext::new(
                System::id(),
                Transfer {
                    from: ctx.accounts.initializer.to_account_info(),
                    to: ctx.accounts.escrow.to_account_info(),
                },
            ),
            ctx.accounts.escrow.amount,
        )?;
        ctx.accounts.escrow.status = EscrowStatus::Funded;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeEscrow<'info> {
    #[account(init, payer = initializer, space = 8 + EscrowV1::INIT_SPACE,
        seeds = [b"escrow", initializer.key().as_ref(), beneficiary.key().as_ref()], bump)]
    pub escrow: Account<'info, EscrowV1>,
    #[account(mut)]
    pub initializer: Signer<'info>,
    // Both parties consent to the fixed-amount escrow at initialization.
    pub beneficiary: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Fund<'info> {
    #[account(mut, has_one = initializer,
        seeds = [b"escrow", initializer.key().as_ref(), escrow.beneficiary.as_ref()], bump)]
    pub escrow: Account<'info, EscrowV1>,
    #[account(mut)]
    pub initializer: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace)]
pub struct EscrowV1 {
    pub initializer: Pubkey,
    pub beneficiary: Pubkey,
    pub amount: u64,
    pub status: EscrowStatus,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, Debug, InitSpace)]
pub enum EscrowStatus {
    Created,
    Funded,
    Released,
    Refunded,
}

#[error_code]
pub enum EscrowError {
    #[msg("Escrow amount must be positive")]
    InvalidAmount,
    #[msg("Escrow parties must be distinct")]
    SameParty,
    #[msg("Escrow is not in the required state")]
    WrongState,
}
