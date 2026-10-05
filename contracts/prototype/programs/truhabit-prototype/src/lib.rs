//! Test-token prototype. This separate program is not the commercial escrow.
use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, TransferChecked};

declare_id!("24nGy1KAx5GSFR2C6xaHnRgKMqYyoNq3LNfu3fFtvNRa");
pub const ORACLE: Pubkey = pubkey!("8m8eyZih5Mjakb8CH5Bxsi7BLp2gbderP6bKuJPm96n4");
pub const MINT: Pubkey = pubkey!("BYa72dJf9S1sw4a4cmESd9DhinH8pQ6fy7gjbPm6VNp3");
pub const RECIPIENT: Pubkey = pubkey!("6dgtm5XMrG6VTSWZGtrpZMWgZHq6mr4JuzBbjBBJTD8e");

#[program]
pub mod truhabit_prototype {
    use super::*;
    pub fn deposit(ctx: Context<Deposit>, args: Terms) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        require!(
            (1_000_000..=50_000_000).contains(&args.amount),
            PrototypeError::InvalidTerms
        );
        require!(
            args.starts >= now - 86400
                && args.starts <= now + 30 * 86400
                && args.ends > args.starts
                && args.ends <= args.starts + 86400,
            PrototypeError::InvalidTerms
        );
        require!(
            args.upload >= args.ends
                && args.upload <= args.ends + 86400
                && args.refund > args.upload
                && args.refund <= args.upload + 86400,
            PrototypeError::InvalidTerms
        );
        require!(now < args.ends, PrototypeError::OutsideWindow);
        let c = &mut ctx.accounts.commitment;
        c.id = args.id;
        c.owner = ctx.accounts.owner.key();
        c.amount = args.amount;
        c.starts = args.starts;
        c.ends = args.ends;
        c.upload = args.upload;
        c.refund = args.refund;
        c.terms_hash = args.terms_hash;
        c.bump = ctx.bumps.commitment;
        c.status = 0;
        token::transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.key(),
                TransferChecked {
                    from: ctx.accounts.source.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.vault.to_account_info(),
                    authority: ctx.accounts.owner.to_account_info(),
                },
            ),
            args.amount,
            6,
        )?;
        Ok(())
    }
    pub fn settle(ctx: Context<Settle>, action: u8) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let c = &ctx.accounts.commitment;
        require!(c.status == 0, PrototypeError::Terminal);
        let actor = ctx.accounts.actor.key();
        match action {
            0 => {
                require_keys_eq!(actor, ORACLE, PrototypeError::Unauthorized);
                require!(
                    now >= c.starts && now < c.refund,
                    PrototypeError::OutsideWindow
                );
            }
            1 => {
                require_keys_eq!(actor, ORACLE, PrototypeError::Unauthorized);
                require!(
                    now > c.upload && now < c.refund,
                    PrototypeError::OutsideWindow
                );
            }
            2 => {
                require_keys_eq!(actor, c.owner, PrototypeError::Unauthorized);
                require!(now < c.starts, PrototypeError::OutsideWindow);
            }
            3 => require!(now >= c.refund, PrototypeError::OutsideWindow),
            _ => return err!(PrototypeError::InvalidTerms),
        }
        let recipient = if action == 1 { RECIPIENT } else { c.owner };
        require_keys_eq!(
            ctx.accounts.destination.owner,
            recipient,
            PrototypeError::Unauthorized
        );
        let seeds: &[&[u8]] = &[b"prototype", c.owner.as_ref(), &c.id, &[c.bump]];
        token::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.key(),
                TransferChecked {
                    from: ctx.accounts.vault.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.destination.to_account_info(),
                    authority: ctx.accounts.commitment.to_account_info(),
                },
                &[seeds],
            ),
            c.amount,
            6,
        )?;
        // Keep the account as a permanent replay tombstone. No close/reinitialize path.
        ctx.accounts.commitment.status = action + 1;
        Ok(())
    }
}
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct Terms {
    pub id: [u8; 16],
    pub amount: u64,
    pub starts: i64,
    pub ends: i64,
    pub upload: i64,
    pub refund: i64,
    pub terms_hash: [u8; 32],
}
#[account]
pub struct Commitment {
    pub id: [u8; 16],
    pub owner: Pubkey,
    pub amount: u64,
    pub starts: i64,
    pub ends: i64,
    pub upload: i64,
    pub refund: i64,
    pub terms_hash: [u8; 32],
    pub bump: u8,
    pub status: u8,
}
impl Commitment {
    pub const SPACE: usize = 8 + 16 + 32 + 8 + 8 * 4 + 32 + 1 + 1;
}
#[derive(Accounts)]
#[instruction(args:Terms)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,
    #[account(address=ORACLE)]
    pub oracle: Signer<'info>,
    #[account(address=MINT,constraint=mint.decimals==6)]
    pub mint: Account<'info, Mint>,
    #[account(init,payer=owner,space=Commitment::SPACE,seeds=[b"prototype",owner.key().as_ref(),&args.id],bump)]
    pub commitment: Account<'info, Commitment>,
    #[account(init,payer=owner,seeds=[b"vault",commitment.key().as_ref()],bump,token::mint=mint,token::authority=commitment)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut,token::mint=mint,token::authority=owner)]
    pub source: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
#[derive(Accounts)]
pub struct Settle<'info> {
    pub actor: Signer<'info>,
    #[account(address=MINT,constraint=mint.decimals==6)]
    pub mint: Account<'info, Mint>,
    #[account(mut,seeds=[b"prototype",commitment.owner.as_ref(),&commitment.id],bump=commitment.bump)]
    pub commitment: Account<'info, Commitment>,
    #[account(mut,seeds=[b"vault",commitment.key().as_ref()],bump,token::mint=mint,token::authority=commitment)]
    pub vault: Account<'info, TokenAccount>,
    #[account(mut,token::mint=mint)]
    pub destination: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
}
#[error_code]
pub enum PrototypeError {
    #[msg("Invalid test-token terms")]
    InvalidTerms,
    #[msg("Outside the agreed time window")]
    OutsideWindow,
    #[msg("Unauthorized actor or destination")]
    Unauthorized,
    #[msg("Commitment already settled")]
    Terminal,
}
