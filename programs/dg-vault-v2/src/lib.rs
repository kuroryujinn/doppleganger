use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod dg_vault_v2 {
    use super::*;

    pub fn migrate(ctx: Context<Migrate>) -> Result<()> {
        #[cfg(not(feature = "r8-repeat-accepted"))]
        let owner = ctx.accounts.vault.owner;
        #[cfg(not(feature = "r8-repeat-accepted"))]
        let balance = ctx.accounts.vault.balance;
        #[cfg(feature = "r8-repeat-accepted")]
        let (owner, balance) = {
            let info = ctx.accounts.vault.to_account_info();
            let data = info.try_borrow_data()?;
            if data.starts_with(VaultV2::DISCRIMINATOR) {
                let existing = VaultV2::try_deserialize(&mut data.as_ref())?;
                require_keys_eq!(
                    existing.owner,
                    ctx.accounts.owner.key(),
                    VaultError::Unauthorized
                );
                // Acceptance-only defect: a repeated migration is accepted as a no-op.
                return Ok(());
            }
            let existing = VaultV1::try_deserialize(&mut data.as_ref())?;
            require_keys_eq!(
                existing.owner,
                ctx.accounts.owner.key(),
                VaultError::Unauthorized
            );
            (existing.owner, existing.balance)
        };
        #[cfg(not(feature = "skip-backing-top-up"))]
        {
            let required_lamports = Rent::get()?
                .minimum_balance(8 + VaultV2::INIT_SPACE)
                .checked_add(balance)
                .ok_or(VaultError::Overflow)?;
            let shortfall =
                required_lamports.saturating_sub(ctx.accounts.vault.to_account_info().lamports());
            if shortfall > 0 {
                system_program::transfer(
                    CpiContext::new(
                        System::id(),
                        Transfer {
                            from: ctx.accounts.owner.to_account_info(),
                            to: ctx.accounts.vault.to_account_info(),
                        },
                    ),
                    shortfall,
                )?;
            }
        }
        // Acceptance-only defect: transaction succeeds but recorded balance changes.
        #[cfg(feature = "r7-broken-balance")]
        let balance = balance
            .checked_sub(50_000)
            .ok_or(VaultError::InsufficientBalance)?;
        let migrated = VaultV2 {
            owner,
            balance,
            withdrawal_limit: 500_000,
        };
        #[cfg(not(feature = "r8-repeat-accepted"))]
        ctx.accounts.vault.migrate(migrated)?;
        #[cfg(feature = "r8-repeat-accepted")]
        {
            ctx.accounts
                .vault
                .to_account_info()
                .resize(8 + VaultV2::INIT_SPACE)?;
            migrated.try_serialize(
                &mut &mut ctx.accounts.vault.to_account_info().try_borrow_mut_data()?[..],
            )?;
        }
        Ok(())
    }

    pub fn deposit(ctx: Context<Operate>, amount: u64) -> Result<()> {
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

    pub fn withdraw(ctx: Context<Operate>, amount: u64) -> Result<()> {
        require!(
            amount <= ctx.accounts.vault.withdrawal_limit,
            VaultError::LimitExceeded
        );
        ctx.accounts.vault.balance = ctx
            .accounts
            .vault
            .balance
            .checked_sub(amount)
            .ok_or(VaultError::InsufficientBalance)?;
        **ctx
            .accounts
            .vault
            .to_account_info()
            .try_borrow_mut_lamports()? -= amount;
        **ctx
            .accounts
            .owner
            .to_account_info()
            .try_borrow_mut_lamports()? += amount;
        Ok(())
    }
}

#[derive(Accounts)]
#[cfg(not(feature = "r8-repeat-accepted"))]
pub struct Migrate<'info> {
    #[account(
        mut,
        seeds = [b"vault", owner.key().as_ref()],
        bump,
        constraint = vault.owner == owner.key() @ VaultError::Unauthorized,
        realloc = 8 + VaultV2::INIT_SPACE,
        realloc::payer = owner,
        realloc::zero = false
    )]
    pub vault: Migration<'info, VaultV1, VaultV2>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[cfg(feature = "r8-repeat-accepted")]
pub struct Migrate<'info> {
    /// CHECK: Test-only dual-version decoding and authority check in the handler.
    #[account(mut, owner = crate::ID, seeds = [b"vault", owner.key().as_ref()], bump)]
    pub vault: UncheckedAccount<'info>,
    #[account(mut)]
    pub owner: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Operate<'info> {
    #[account(mut, has_one = owner, seeds = [b"vault", owner.key().as_ref()], bump)]
    pub vault: Account<'info, VaultV2>,
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

#[account]
#[derive(InitSpace)]
pub struct VaultV2 {
    pub owner: Pubkey,
    pub balance: u64,
    pub withdrawal_limit: u64,
}

#[error_code]
pub enum VaultError {
    #[msg("Balance overflow")]
    Overflow,
    #[msg("Insufficient balance")]
    InsufficientBalance,
    #[msg("Withdrawal limit exceeded")]
    LimitExceeded,
    #[msg("Unauthorized migration")]
    Unauthorized,
}
