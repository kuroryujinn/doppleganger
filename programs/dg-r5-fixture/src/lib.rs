use anchor_lang::prelude::*;

declare_id!("9xQeWvG816bUx9EPfEZzU4kL9fVMKcVx5yFvTJTQHnN");

#[program]
pub mod dg_r5_fixture {
    use super::*;
    pub fn initialize(ctx: Context<Initialize>, _tag: u8, value: u64) -> Result<()> {
        ctx.accounts.record.authority = ctx.accounts.authority.key();
        ctx.accounts.record.value = value;
        Ok(())
    }
    pub fn advance(ctx: Context<Advance>, expected: u64, next: u64) -> Result<()> {
        require!(
            ctx.accounts.record.value == expected,
            FixtureError::OutOfOrder
        );
        ctx.accounts.record.value = next;
        Ok(())
    }
    pub fn corrupt(ctx: Context<Corrupt>) -> Result<()> {
        ctx.accounts.record.try_borrow_mut_data()?[0] = 255;
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(tag: u8)]
pub struct Initialize<'info> {
    #[account(init, payer = authority, space = 8 + Record::INIT_SPACE, seeds = [b"record", authority.key().as_ref(), &[tag]], bump)]
    pub record: Account<'info, Record>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct Advance<'info> {
    #[account(mut, has_one = authority)]
    pub record: Account<'info, Record>,
    pub authority: Signer<'info>,
}
#[derive(Accounts)]
pub struct Corrupt<'info> {
    /// CHECK: deliberate malformed-state fixture, restricted to program ownership.
    #[account(mut, owner = crate::ID)]
    pub record: UncheckedAccount<'info>,
}
#[account]
#[derive(InitSpace)]
pub struct Record {
    pub authority: Pubkey,
    pub value: u64,
}
#[error_code]
pub enum FixtureError {
    #[msg("Unexpected prior value")]
    OutOfOrder,
}
