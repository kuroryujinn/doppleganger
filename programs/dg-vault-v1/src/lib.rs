use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod dg_vault_v1 {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        ctx.accounts.vault.owner = ctx.accounts.owner.key();
        ctx.accounts.vault.balance = 0;
        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        let next = ctx
            .accounts
            .vault
            .balance
            .checked_add(amount)
            .ok_or(VaultError::Overflow)?;
        system_program::transfer(
            CpiContext::new(
                System::id(),
                Transfer {
                    from: ctx.accounts.owner.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                },
            ),
            amount,
        )?;
        ctx.accounts.vault.balance = next;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = owner, space = 8 + VaultV1::INIT_SPACE, seeds = [b"vault", owner.key().as_ref()], bump)]
    pub vault: Account<'info, VaultV1>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut, has_one = owner, seeds = [b"vault", owner.key().as_ref()], bump)]
    pub vault: Account<'info, VaultV1>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace)]
pub struct VaultV1 {
    pub owner: Pubkey,
    pub balance: u64,
}

#[error_code]
pub enum VaultError {
    #[msg("Balance overflow")]
    Overflow,
}
