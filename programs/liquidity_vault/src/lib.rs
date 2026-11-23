use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Burn, Mint, MintTo, Token, TokenAccount, TransferChecked},
};
pub mod strategy;
pub use strategy::*;

declare_id!("GAKcXHFZyJxktyxGrzZptDoqA8GUtCeAFuhmEtjFmyqZ");

#[program]
pub mod liquidity_vault {
    use super::*;

    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        let vault = &mut ctx.accounts.vault;
        vault.admin = ctx.accounts.admin.key();
        vault.base_mint = ctx.accounts.base_mint.key();
        vault.share_mint = ctx.accounts.share_mint.key();
        vault.bump = ctx.bumps.vault;
        vault.share_bump = ctx.bumps.share_mint;
        vault.strategy_authority_bump = ctx.bumps.strategy_authority;
        vault.strategy_pool = Pubkey::default();
        vault.quote_mint = Pubkey::default();
        vault.position_nft_mint = Pubkey::default();
        vault.position_nft_mint_bump = 0;
        vault.position_nonce = 0;
        vault.strategy_locked = false;
        require!(
            ctx.accounts.strategy_authority.data_is_empty(),
            VaultError::InvalidStrategyState
        );
        require_keys_eq!(
            *ctx.accounts.strategy_authority.owner,
            ctx.accounts.system_program.key(),
            VaultError::InvalidStrategyState
        );
        let rent_funding = Rent::get()?
            .minimum_balance(0)
            .saturating_sub(ctx.accounts.strategy_authority.lamports());
        if rent_funding > 0 {
            anchor_lang::solana_program::program::invoke(
                &anchor_lang::solana_program::system_instruction::transfer(
                    &ctx.accounts.admin.key(),
                    &ctx.accounts.strategy_authority.key(),
                    rent_funding,
                ),
                &[
                    ctx.accounts.admin.to_account_info(),
                    ctx.accounts.strategy_authority.to_account_info(),
                    ctx.accounts.system_program.to_account_info(),
                ],
            )?;
        }
        Ok(())
    }

    pub fn open_position(ctx: Context<OpenPosition>) -> Result<()> {
        let position = &mut ctx.accounts.position;
        position.owner = ctx.accounts.user.key();
        position.vault = ctx.accounts.vault.key();
        position.bump = ctx.bumps.position;
        position.initialized = true;
        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(
            !ctx.accounts.vault.strategy_locked,
            VaultError::StrategyLocked
        );
        require!(amount > 0, VaultError::ZeroAmount);

        let assets_before = ctx.accounts.vault_token_account.amount;
        let shares_before = ctx.accounts.share_mint.supply;
        let shares = calculate_shares(amount, assets_before, shares_before)?;
        require!(shares > 0, VaultError::DepositTooSmall);

        let transfer_accounts = TransferChecked {
            from: ctx.accounts.user_base_account.to_account_info(),
            mint: ctx.accounts.base_mint.to_account_info(),
            to: ctx.accounts.vault_token_account.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        token::transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                transfer_accounts,
            ),
            amount,
            ctx.accounts.base_mint.decimals,
        )?;

        let bump = [ctx.accounts.vault.bump];
        let vault_seeds: &[&[u8]] = &[b"vault", ctx.accounts.vault.base_mint.as_ref(), &bump];
        let mint_accounts = MintTo {
            mint: ctx.accounts.share_mint.to_account_info(),
            to: ctx.accounts.user_share_account.to_account_info(),
            authority: ctx.accounts.vault.to_account_info(),
        };
        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                mint_accounts,
                &[vault_seeds],
            ),
            shares,
        )?;

        let position = &mut ctx.accounts.position;
        position.total_deposited = position
            .total_deposited
            .checked_add(amount)
            .ok_or(VaultError::ArithmeticOverflow)?;
        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, shares: u64) -> Result<()> {
        require!(
            !ctx.accounts.vault.strategy_locked,
            VaultError::StrategyLocked
        );
        require!(shares > 0, VaultError::ZeroAmount);
        require!(
            shares <= ctx.accounts.user_share_account.amount,
            VaultError::InsufficientShares
        );

        let supply = ctx.accounts.share_mint.supply;
        require!(supply > 0, VaultError::EmptyShareSupply);
        let assets = u64::try_from(
            u128::from(shares)
                .checked_mul(u128::from(ctx.accounts.vault_token_account.amount))
                .ok_or(VaultError::ArithmeticOverflow)?
                / u128::from(supply),
        )
        .map_err(|_| VaultError::ArithmeticOverflow)?;
        require!(assets > 0, VaultError::WithdrawTooSmall);

        let burn_accounts = Burn {
            mint: ctx.accounts.share_mint.to_account_info(),
            from: ctx.accounts.user_share_account.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        };
        token::burn(
            CpiContext::new(ctx.accounts.token_program.to_account_info(), burn_accounts),
            shares,
        )?;

        let bump = [ctx.accounts.vault.strategy_authority_bump];
        let vault_key = ctx.accounts.vault.key();
        let authority_seeds: &[&[u8]] = &[b"strategy-authority", vault_key.as_ref(), &bump];
        let transfer_accounts = TransferChecked {
            from: ctx.accounts.vault_token_account.to_account_info(),
            mint: ctx.accounts.base_mint.to_account_info(),
            to: ctx.accounts.user_base_account.to_account_info(),
            authority: ctx.accounts.strategy_authority.to_account_info(),
        };
        token::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                transfer_accounts,
                &[authority_seeds],
            ),
            assets,
            ctx.accounts.base_mint.decimals,
        )?;

        Ok(())
    }

    pub fn configure_strategy(ctx: Context<ConfigureStrategy>) -> Result<()> {
        strategy::configure_strategy(ctx)
    }

    pub fn begin_strategy(ctx: Context<BeginStrategy>, funding_lamports: u64) -> Result<()> {
        strategy::begin_strategy(ctx, funding_lamports)
    }

    pub fn swap<'a, 'b, 'c: 'info, 'info>(
        ctx: Context<'a, 'b, 'c, 'info, Swap<'info>>,
        input_is_base: bool,
        amount: u64,
        min_output: u64,
    ) -> Result<()> {
        strategy::swap(ctx, input_is_base, amount, min_output)
    }

    pub fn open_clmm_position<'a, 'b, 'c: 'info, 'info>(
        ctx: Context<'a, 'b, 'c, 'info, OpenClmmPosition<'info>>,
        tick_lower_index: i32,
        tick_upper_index: i32,
        tick_array_lower_start_index: i32,
        tick_array_upper_start_index: i32,
        liquidity: u128,
        amount_0_max: u64,
        amount_1_max: u64,
    ) -> Result<()> {
        strategy::open_clmm_position(
            ctx,
            tick_lower_index,
            tick_upper_index,
            tick_array_lower_start_index,
            tick_array_upper_start_index,
            liquidity,
            amount_0_max,
            amount_1_max,
        )
    }

    pub fn decrease_liquidity<'a, 'b, 'c: 'info, 'info>(
        ctx: Context<'a, 'b, 'c, 'info, DecreaseLiquidity<'info>>,
        tick_array_lower_start_index: i32,
        tick_array_upper_start_index: i32,
        liquidity: u128,
        amount_0_min: u64,
        amount_1_min: u64,
    ) -> Result<()> {
        strategy::decrease_liquidity(
            ctx,
            tick_array_lower_start_index,
            tick_array_upper_start_index,
            liquidity,
            amount_0_min,
            amount_1_min,
        )
    }

    pub fn collect_position_fees<'a, 'b, 'c: 'info, 'info>(
        ctx: Context<'a, 'b, 'c, 'info, DecreaseLiquidity<'info>>,
        tick_array_lower_start_index: i32,
        tick_array_upper_start_index: i32,
    ) -> Result<()> {
        strategy::collect_position_fees(
            ctx,
            tick_array_lower_start_index,
            tick_array_upper_start_index,
        )
    }

    pub fn close_clmm_position<'a, 'b, 'c: 'info, 'info>(
        ctx: Context<'a, 'b, 'c, 'info, CloseClmmPosition<'info>>,
    ) -> Result<()> {
        strategy::close_clmm_position(ctx)
    }

    pub fn end_strategy(ctx: Context<EndStrategy>) -> Result<()> {
        strategy::end_strategy(ctx)
    }
}

fn calculate_shares(amount: u64, assets: u64, supply: u64) -> Result<u64> {
    if supply == 0 {
        require!(assets == 0, VaultError::InconsistentVault);
        return Ok(amount);
    }
    require!(assets > 0, VaultError::InconsistentVault);
    u64::try_from(
        u128::from(amount)
            .checked_mul(u128::from(supply))
            .ok_or(VaultError::ArithmeticOverflow)?
            / u128::from(assets),
    )
    .map_err(|_| VaultError::ArithmeticOverflow.into())
}

#[derive(Accounts)]
pub struct OpenPosition<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(
        seeds = [b"vault", vault.base_mint.as_ref()],
        bump = vault.bump
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        init,
        payer = user,
        space = 8 + Position::INIT_SPACE,
        seeds = [b"position", vault.key().as_ref(), user.key().as_ref()],
        bump
    )]
    pub position: Account<'info, Position>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    pub base_mint: Account<'info, Mint>,
    #[account(
        init,
        payer = admin,
        space = 8 + Vault::INIT_SPACE,
        seeds = [b"vault", base_mint.key().as_ref()],
        bump
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        init,
        payer = admin,
        seeds = [b"share", vault.key().as_ref()],
        bump,
        mint::decimals = base_mint.decimals,
        mint::authority = vault
    )]
    pub share_mint: Account<'info, Mint>,
    /// CHECK: System-owned PDA used as the token authority and Raydium CPI payer.
    #[account(
        mut,
        seeds = [b"strategy-authority", vault.key().as_ref()],
        bump
    )]
    pub strategy_authority: UncheckedAccount<'info>,
    #[account(
        init,
        payer = admin,
        associated_token::mint = base_mint,
        associated_token::authority = strategy_authority
    )]
    pub vault_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(
        seeds = [b"vault", base_mint.key().as_ref()],
        bump = vault.bump,
        has_one = base_mint,
        has_one = share_mint
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        seeds = [b"strategy-authority", vault.key().as_ref()],
        bump = vault.strategy_authority_bump
    )]
    pub strategy_authority: SystemAccount<'info>,
    pub base_mint: Account<'info, Mint>,
    #[account(
        mut,
        seeds = [b"share", vault.key().as_ref()],
        bump = vault.share_bump,
        mint::authority = vault
    )]
    pub share_mint: Account<'info, Mint>,
    #[account(
        mut,
        associated_token::mint = base_mint,
        associated_token::authority = strategy_authority
    )]
    pub vault_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = user_base_account.owner == user.key(),
        constraint = user_base_account.mint == base_mint.key()
    )]
    pub user_base_account: Account<'info, TokenAccount>,
    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = share_mint,
        associated_token::authority = user
    )]
    pub user_share_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [b"position", vault.key().as_ref(), user.key().as_ref()],
        bump = position.bump,
        constraint = position.initialized,
        constraint = position.owner == user.key(),
        has_one = vault
    )]
    pub position: Account<'info, Position>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    pub user: Signer<'info>,
    #[account(
        seeds = [b"vault", base_mint.key().as_ref()],
        bump = vault.bump,
        has_one = base_mint,
        has_one = share_mint
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        seeds = [b"strategy-authority", vault.key().as_ref()],
        bump = vault.strategy_authority_bump
    )]
    pub strategy_authority: SystemAccount<'info>,
    pub base_mint: Account<'info, Mint>,
    #[account(
        mut,
        seeds = [b"share", vault.key().as_ref()],
        bump = vault.share_bump,
        mint::authority = vault
    )]
    pub share_mint: Account<'info, Mint>,
    #[account(
        mut,
        associated_token::mint = base_mint,
        associated_token::authority = strategy_authority
    )]
    pub vault_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = user_base_account.owner == user.key(),
        constraint = user_base_account.mint == base_mint.key()
    )]
    pub user_base_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        associated_token::mint = share_mint,
        associated_token::authority = user
    )]
    pub user_share_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [b"position", vault.key().as_ref(), user.key().as_ref()],
        bump = position.bump,
        constraint = position.owner == user.key(),
        has_one = vault
    )]
    pub position: Account<'info, Position>,
    pub token_program: Program<'info, Token>,
}

#[account]
#[derive(InitSpace)]
pub struct Vault {
    pub admin: Pubkey,
    pub base_mint: Pubkey,
    pub share_mint: Pubkey,
    pub bump: u8,
    pub share_bump: u8,
    pub strategy_authority_bump: u8,
    pub strategy_pool: Pubkey,
    pub quote_mint: Pubkey,
    pub position_nft_mint: Pubkey,
    pub position_nft_mint_bump: u8,
    pub position_nonce: u64,
    pub strategy_locked: bool,
}

#[account]
#[derive(InitSpace)]
pub struct Position {
    pub owner: Pubkey,
    pub vault: Pubkey,
    pub total_deposited: u64,
    pub bump: u8,
    pub initialized: bool,
}

#[error_code]
pub enum VaultError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Deposit is too small to mint at least one share")]
    DepositTooSmall,
    #[msg("Withdrawal rounds down to zero base tokens")]
    WithdrawTooSmall,
    #[msg("Share supply is empty")]
    EmptyShareSupply,
    #[msg("The account does not hold enough shares")]
    InsufficientShares,
    #[msg("Vault token balance and share supply are inconsistent")]
    InconsistentVault,
    #[msg("Arithmetic overflow")]
    ArithmeticOverflow,
    #[msg("Deposits and withdrawals are disabled while the strategy is active")]
    StrategyLocked,
    #[msg("The strategy is already active or not configured")]
    InvalidStrategyState,
    #[msg("The supplied Raydium pool does not match the configured token pair")]
    InvalidStrategyPool,
    #[msg("The supplied token account is not controlled by the strategy PDA")]
    InvalidStrategyTokenAccount,
    #[msg("The CLMM position is not open or does not belong to this vault")]
    InvalidClmmPosition,
    #[msg("The strategy must close its position and convert quote assets before resuming")]
    StrategyNotLiquidated,
}

#[cfg(test)]
mod tests {
    use super::calculate_shares;

    #[test]
    fn first_deposit_mints_one_share_per_base_token() {
        assert_eq!(calculate_shares(250, 0, 0).unwrap(), 250);
    }

    #[test]
    fn later_deposit_preserves_existing_share_value() {
        assert_eq!(calculate_shares(110, 1_100, 1_000).unwrap(), 100);
    }

    #[test]
    fn rejects_zero_assets_with_existing_shares() {
        assert!(calculate_shares(10, 0, 1).is_err());
    }
}
