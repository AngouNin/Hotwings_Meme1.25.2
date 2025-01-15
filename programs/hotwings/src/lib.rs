use anchor_lang::prelude::*;

declare_id!("6vxBssG3FvWset4jv3STQGGnq3mTqkkD2BSbYC5s7j89");

#[program]
pub mod hotwings {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
