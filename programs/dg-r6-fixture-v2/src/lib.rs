use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Transfer};
declare_id!("9xQeWvG816bUx9EPfEZzU4kL9fVMKcVx5yFvTJTQHnN");

#[program]
pub mod dg_r6_fixture_v2 {
    use super::*;
    pub fn migrate(ctx: Context<Migrate>, mode: u8, version: u8, value: u64) -> Result<()> {
        let authority = ctx.accounts.record.authority;
        ctx.accounts.record.migrate(RecordV2 {
            authority,
            value,
            version,
        })?;
        if mode == 2 {
            // Ensure a real byte write and transfer occur before the deliberate error.
            ctx.accounts
                .record
                .to_account_info()
                .try_borrow_mut_data()?[40] = 99;
            system_program::transfer(
                CpiContext::new(
                    System::id(),
                    Transfer {
                        from: ctx.accounts.authority.to_account_info(),
                        to: ctx.accounts.record.to_account_info(),
                    },
                ),
                12345,
            )?;
            msg!("attempted write, realloc and transfer before failure");
            return err!(FixtureError::Deliberate);
        }
        Ok(())
    }
    pub fn reject(_ctx: Context<Read>) -> Result<()> {
        msg!("deliberate failure before mutation");
        err!(FixtureError::Deliberate)
    }
    pub fn create(ctx: Context<Create>) -> Result<()> {
        ctx.accounts.receipt.set_inner(RecordV2 {
            authority: ctx.accounts.authority.key(),
            value: 77,
            version: 2,
        });
        Ok(())
    }
    pub fn close(_ctx: Context<Close>) -> Result<()> {
        Ok(())
    }
    pub fn arguments(_ctx: Context<Read>, payload: Payload) -> Result<()> {
        msg!(
            "wide={} signed={} count={}",
            payload.wide,
            payload.signed,
            payload.values.len()
        );
        Ok(())
    }
    pub fn operate(ctx: Context<Operate>, value: u64, fail: bool) -> Result<()> {
        require!(!fail, FixtureError::Deliberate);
        ctx.accounts.record.value = value;
        Ok(())
    }
    pub fn noop(_ctx: Context<Read>) -> Result<()> {
        Ok(())
    }
}
#[derive(Accounts)]
pub struct Operate<'info> {
    #[account(mut, has_one = authority)]
    pub record: Account<'info, RecordV2>,
    pub authority: Signer<'info>,
}
#[derive(Accounts)]
pub struct Migrate<'info> {
    #[account(mut, constraint = record.authority == authority.key(), realloc = 8 + RecordV2::INIT_SPACE, realloc::payer = authority, realloc::zero = false)]
    pub record: Migration<'info, RecordV1, RecordV2>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct Read<'info> {
    #[account(has_one = authority)]
    pub record: Account<'info, RecordV1>,
    pub authority: Signer<'info>,
}
#[derive(Accounts)]
pub struct Create<'info> {
    #[account(init, payer = authority, space = 8 + RecordV2::INIT_SPACE, seeds = [b"receipt", authority.key().as_ref()], bump)]
    pub receipt: Account<'info, RecordV2>,
    #[account(mut)]
    pub authority: Signer<'info>,
    pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct Close<'info> {
    #[account(mut, has_one = authority, close = authority)]
    pub record: Account<'info, RecordV1>,
    #[account(mut)]
    pub authority: Signer<'info>,
}
#[account]
#[derive(InitSpace)]
pub struct RecordV1 {
    pub authority: Pubkey,
    pub value: u64,
}
#[account]
#[derive(InitSpace)]
pub struct RecordV2 {
    pub authority: Pubkey,
    pub value: u64,
    pub version: u8,
}
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct Payload {
    pub wide: u128,
    pub signed: i128,
    pub nested: Nested,
    pub choice: Choice,
    pub values: Vec<u64>,
    pub maybe: Option<u64>,
    pub array: [u8; 2],
    pub pair: (u8, u16),
}
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct Nested {
    pub flag: bool,
}
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub enum Choice {
    Empty,
    Number(u64),
}
#[error_code]
pub enum FixtureError {
    #[msg("Deliberate fixture failure")]
    Deliberate,
}
