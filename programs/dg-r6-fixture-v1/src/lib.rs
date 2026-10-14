use anchor_lang::prelude::*;
declare_id!("9xQeWvG816bUx9EPfEZzU4kL9fVMKcVx5yFvTJTQHnN");

#[program]
pub mod dg_r6_fixture_v1 {
    use super::*;
    pub fn initialize(ctx: Context<Initialize>, value: u64) -> Result<()> {
        ctx.accounts.record.set_inner(RecordV1 {
            authority: ctx.accounts.authority.key(),
            value,
        });
        Ok(())
    }
    pub fn advance(ctx: Context<Advance>, value: u64) -> Result<()> {
        ctx.accounts.record.value = value;
        Ok(())
    }
}
#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(init, payer = authority, space = 8 + RecordV1::INIT_SPACE, seeds = [b"record", authority.key().as_ref()], bump)]
    pub record: Account<'info, RecordV1>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct Advance<'info> {
    #[account(mut, has_one = authority)]
    pub record: Account<'info, RecordV1>,
    pub authority: Signer<'info>,
}
#[account]
#[derive(InitSpace)]
pub struct RecordV1 {
    pub authority: Pubkey,
    pub value: u64,
}
