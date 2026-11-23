use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    memo::Memo,
    metadata::Metadata,
    token::{Mint, Token, TokenAccount},
    token_2022::Token2022,
};
use raydium_clmm_cpi::{
    cpi,
    program::RaydiumClmm,
    states::{
        AmmConfig, ObservationState, PersonalPositionState, PoolState, TickArrayState, POOL_SEED,
        POSITION_SEED, TICK_ARRAY_SEED,
    },
};

use crate::{Vault, VaultError};

pub(super) fn configure_strategy(ctx: Context<ConfigureStrategy>) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    require!(
        vault.strategy_pool == Pubkey::default() && !vault.strategy_locked,
        VaultError::InvalidStrategyState
    );
    require_keys_neq!(
        ctx.accounts.base_mint.key(),
        ctx.accounts.quote_mint.key(),
        VaultError::InvalidStrategyPool
    );

    let pool = ctx.accounts.pool_state.load()?;
    validate_pool_mints(vault, &pool, ctx.accounts.quote_mint.key())?;
    let (pool_address, _) = Pubkey::find_program_address(
        &[
            POOL_SEED.as_bytes(),
            pool.amm_config.as_ref(),
            pool.token_mint_0.as_ref(),
            pool.token_mint_1.as_ref(),
        ],
        &ctx.accounts.clmm_program.key(),
    );
    require_keys_eq!(
        pool_address,
        ctx.accounts.pool_state.key(),
        VaultError::InvalidStrategyPool
    );

    vault.strategy_pool = ctx.accounts.pool_state.key();
    vault.quote_mint = ctx.accounts.quote_mint.key();
    Ok(())
}

pub(super) fn begin_strategy(ctx: Context<BeginStrategy>, funding_lamports: u64) -> Result<()> {
    let vault = &mut ctx.accounts.vault;
    require!(
        vault.strategy_pool != Pubkey::default()
            && !vault.strategy_locked
            && vault.position_nft_mint == Pubkey::default(),
        VaultError::InvalidStrategyState
    );

    if funding_lamports > 0 {
        anchor_lang::solana_program::program::invoke(
            &anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.admin.key(),
                &ctx.accounts.strategy_authority.key(),
                funding_lamports,
            ),
            &[
                ctx.accounts.admin.to_account_info(),
                ctx.accounts.strategy_authority.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
        )?;
    }

    vault.strategy_locked = true;
    Ok(())
}

pub(super) fn swap<'a, 'b, 'c: 'info, 'info>(
    ctx: Context<'a, 'b, 'c, 'info, Swap<'info>>,
    input_is_base: bool,
    amount: u64,
    min_output: u64,
) -> Result<()> {
    require!(amount > 0 && min_output > 0, VaultError::ZeroAmount);
    let pool = ctx.accounts.pool_state.load()?;
    validate_configured_pool(&ctx.accounts.vault, &pool, ctx.accounts.quote_mint.key())?;

    let base_is_token_0 = pool.token_mint_0 == ctx.accounts.base_mint.key();
    let input_is_token_0 = input_is_base == base_is_token_0;
    let (
        input_token_account,
        output_token_account,
        input_vault,
        output_vault,
        input_mint,
        output_mint,
    ) = if input_is_token_0 {
        (
            ctx.accounts.base_token_account.to_account_info(),
            ctx.accounts.quote_token_account.to_account_info(),
            ctx.accounts.pool_vault_0.to_account_info(),
            ctx.accounts.pool_vault_1.to_account_info(),
            ctx.accounts.mint_0.to_account_info(),
            ctx.accounts.mint_1.to_account_info(),
        )
    } else {
        (
            ctx.accounts.quote_token_account.to_account_info(),
            ctx.accounts.base_token_account.to_account_info(),
            ctx.accounts.pool_vault_1.to_account_info(),
            ctx.accounts.pool_vault_0.to_account_info(),
            ctx.accounts.mint_1.to_account_info(),
            ctx.accounts.mint_0.to_account_info(),
        )
    };
    drop(pool);

    let bump = [ctx.accounts.vault.strategy_authority_bump];
    let vault_key = ctx.accounts.vault.key();
    let signer_seeds: &[&[u8]] = &[b"strategy-authority", vault_key.as_ref(), &bump];
    let signer_seeds_set: &[&[&[u8]]] = &[signer_seeds];
    let cpi_accounts = cpi::accounts::SwapSingleV2 {
        payer: ctx.accounts.strategy_authority.to_account_info(),
        amm_config: ctx.accounts.amm_config.to_account_info(),
        pool_state: ctx.accounts.pool_state.to_account_info(),
        input_token_account,
        output_token_account,
        input_vault,
        output_vault,
        observation_state: ctx.accounts.observation_state.to_account_info(),
        token_program: ctx.accounts.token_program.to_account_info(),
        token_program_2022: ctx.accounts.token_program_2022.to_account_info(),
        memo_program: ctx.accounts.memo_program.to_account_info(),
        input_vault_mint: input_mint,
        output_vault_mint: output_mint,
    };
    let cpi_context = CpiContext::new_with_signer(
        ctx.accounts.clmm_program.to_account_info(),
        cpi_accounts,
        signer_seeds_set,
    )
    .with_remaining_accounts(ctx.remaining_accounts.to_vec());
    cpi::swap_v2(cpi_context, amount, min_output, 0, true)?;
    Ok(())
}

pub(super) fn open_clmm_position<'a, 'b, 'c: 'info, 'info>(
    ctx: Context<'a, 'b, 'c, 'info, OpenClmmPosition<'info>>,
    tick_lower_index: i32,
    tick_upper_index: i32,
    tick_array_lower_start_index: i32,
    tick_array_upper_start_index: i32,
    liquidity: u128,
    amount_0_max: u64,
    amount_1_max: u64,
) -> Result<()> {
    require!(
        ctx.accounts.vault.strategy_locked
            && ctx.accounts.vault.position_nft_mint == Pubkey::default(),
        VaultError::InvalidStrategyState
    );
    require!(
        tick_lower_index < tick_upper_index && liquidity > 0,
        VaultError::InvalidClmmPosition
    );
    let pool = ctx.accounts.pool_state.load()?;
    validate_configured_pool(&ctx.accounts.vault, &pool, ctx.accounts.quote_mint.key())?;
    let pool_mint_0 = pool.token_mint_0;
    drop(pool);

    let base_is_token_0 = pool_mint_0 == ctx.accounts.base_mint.key();
    let (token_account_0, token_account_1) = if base_is_token_0 {
        (
            ctx.accounts.base_token_account.to_account_info(),
            ctx.accounts.quote_token_account.to_account_info(),
        )
    } else {
        (
            ctx.accounts.quote_token_account.to_account_info(),
            ctx.accounts.base_token_account.to_account_info(),
        )
    };
    validate_position_ata(
        ctx.accounts.position_nft_account.key(),
        ctx.accounts.strategy_authority.key(),
        ctx.accounts.position_nft_mint.key(),
        ctx.accounts.token_program.key(),
        ctx.accounts.associated_token_program.key(),
    )?;
    validate_metadata_address(
        ctx.accounts.metadata_account.key(),
        ctx.accounts.position_nft_mint.key(),
        ctx.accounts.metadata_program.key(),
    )?;

    let vault_key = ctx.accounts.vault.key();
    let authority_bump = [ctx.accounts.vault.strategy_authority_bump];
    let authority_seeds: &[&[u8]] = &[b"strategy-authority", vault_key.as_ref(), &authority_bump];
    let nonce = ctx.accounts.vault.position_nonce.to_le_bytes();
    let mint_bump = [ctx.bumps.position_nft_mint];
    let mint_seeds: &[&[u8]] = &[
        b"clmm-position-mint",
        vault_key.as_ref(),
        &nonce,
        &mint_bump,
    ];
    let signer_seeds: &[&[&[u8]]] = &[authority_seeds, mint_seeds];
    let cpi_accounts = cpi::accounts::OpenPositionV2 {
        payer: ctx.accounts.strategy_authority.to_account_info(),
        position_nft_owner: ctx.accounts.strategy_authority.to_account_info(),
        position_nft_mint: ctx.accounts.position_nft_mint.to_account_info(),
        position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
        metadata_account: ctx.accounts.metadata_account.to_account_info(),
        pool_state: ctx.accounts.pool_state.to_account_info(),
        protocol_position: ctx.accounts.protocol_position.to_account_info(),
        tick_array_lower: ctx.accounts.tick_array_lower.to_account_info(),
        tick_array_upper: ctx.accounts.tick_array_upper.to_account_info(),
        personal_position: ctx.accounts.personal_position.to_account_info(),
        token_account_0,
        token_account_1,
        token_vault_0: ctx.accounts.pool_vault_0.to_account_info(),
        token_vault_1: ctx.accounts.pool_vault_1.to_account_info(),
        rent: ctx.accounts.rent.to_account_info(),
        system_program: ctx.accounts.system_program.to_account_info(),
        token_program: ctx.accounts.token_program.to_account_info(),
        associated_token_program: ctx.accounts.associated_token_program.to_account_info(),
        metadata_program: ctx.accounts.metadata_program.to_account_info(),
        token_program_2022: ctx.accounts.token_program_2022.to_account_info(),
        vault_0_mint: ctx.accounts.mint_0.to_account_info(),
        vault_1_mint: ctx.accounts.mint_1.to_account_info(),
    };
    let cpi_context = CpiContext::new_with_signer(
        ctx.accounts.clmm_program.to_account_info(),
        cpi_accounts,
        signer_seeds,
    )
    .with_remaining_accounts(ctx.remaining_accounts.to_vec());
    cpi::open_position_v2(
        cpi_context,
        tick_lower_index,
        tick_upper_index,
        tick_array_lower_start_index,
        tick_array_upper_start_index,
        liquidity,
        amount_0_max,
        amount_1_max,
        false,
        None,
    )?;

    ctx.accounts.vault.position_nft_mint = ctx.accounts.position_nft_mint.key();
    ctx.accounts.vault.position_nft_mint_bump = ctx.bumps.position_nft_mint;
    Ok(())
}

pub(super) fn decrease_liquidity<'a, 'b, 'c: 'info, 'info>(
    ctx: Context<'a, 'b, 'c, 'info, DecreaseLiquidity<'info>>,
    _tick_array_lower_start_index: i32,
    _tick_array_upper_start_index: i32,
    liquidity: u128,
    amount_0_min: u64,
    amount_1_min: u64,
) -> Result<()> {
    require!(liquidity > 0, VaultError::ZeroAmount);
    require!(
        liquidity <= ctx.accounts.personal_position.liquidity,
        VaultError::InvalidClmmPosition
    );
    invoke_decrease_liquidity(ctx, liquidity, amount_0_min, amount_1_min)
}

pub(super) fn collect_position_fees<'a, 'b, 'c: 'info, 'info>(
    ctx: Context<'a, 'b, 'c, 'info, DecreaseLiquidity<'info>>,
    _tick_array_lower_start_index: i32,
    _tick_array_upper_start_index: i32,
) -> Result<()> {
    invoke_decrease_liquidity(ctx, 0, 0, 0)
}

fn invoke_decrease_liquidity<'a, 'b, 'c: 'info, 'info>(
    ctx: Context<'a, 'b, 'c, 'info, DecreaseLiquidity<'info>>,
    liquidity: u128,
    amount_0_min: u64,
    amount_1_min: u64,
) -> Result<()> {
    require!(
        ctx.accounts.vault.strategy_locked,
        VaultError::InvalidStrategyState
    );
    require_keys_eq!(
        ctx.accounts.position_nft_mint.key(),
        ctx.accounts.vault.position_nft_mint,
        VaultError::InvalidClmmPosition
    );
    validate_configured_pool(
        &ctx.accounts.vault,
        &*ctx.accounts.pool_state.load()?,
        ctx.accounts.quote_mint.key(),
    )?;
    validate_personal_position(
        &ctx.accounts.personal_position,
        ctx.accounts.pool_state.key(),
        ctx.accounts.position_nft_mint.key(),
    )?;
    validate_vault_token_owner(
        &ctx.accounts.base_token_account,
        &ctx.accounts.quote_token_account,
        ctx.accounts.strategy_authority.key(),
    )?;

    let vault_key = ctx.accounts.vault.key();
    let bump = [ctx.accounts.vault.strategy_authority_bump];
    let signer_seeds: &[&[u8]] = &[b"strategy-authority", vault_key.as_ref(), &bump];
    let signer_seeds_set: &[&[&[u8]]] = &[signer_seeds];
    let pool_mint_0 = ctx.accounts.mint_0.key();
    let (recipient_token_account_0, recipient_token_account_1) =
        if pool_mint_0 == ctx.accounts.base_mint.key() {
            (
                ctx.accounts.base_token_account.to_account_info(),
                ctx.accounts.quote_token_account.to_account_info(),
            )
        } else {
            (
                ctx.accounts.quote_token_account.to_account_info(),
                ctx.accounts.base_token_account.to_account_info(),
            )
        };
    let cpi_accounts = cpi::accounts::DecreaseLiquidityV2 {
        nft_owner: ctx.accounts.strategy_authority.to_account_info(),
        nft_account: ctx.accounts.position_nft_account.to_account_info(),
        personal_position: ctx.accounts.personal_position.to_account_info(),
        pool_state: ctx.accounts.pool_state.to_account_info(),
        protocol_position: ctx.accounts.protocol_position.to_account_info(),
        token_vault_0: ctx.accounts.pool_vault_0.to_account_info(),
        token_vault_1: ctx.accounts.pool_vault_1.to_account_info(),
        tick_array_lower: ctx.accounts.tick_array_lower.to_account_info(),
        tick_array_upper: ctx.accounts.tick_array_upper.to_account_info(),
        recipient_token_account_0,
        recipient_token_account_1,
        token_program: ctx.accounts.token_program.to_account_info(),
        token_program_2022: ctx.accounts.token_program_2022.to_account_info(),
        memo_program: ctx.accounts.memo_program.to_account_info(),
        vault_0_mint: ctx.accounts.mint_0.to_account_info(),
        vault_1_mint: ctx.accounts.mint_1.to_account_info(),
    };
    let cpi_context = CpiContext::new_with_signer(
        ctx.accounts.clmm_program.to_account_info(),
        cpi_accounts,
        signer_seeds_set,
    )
    .with_remaining_accounts(ctx.remaining_accounts.to_vec());
    cpi::decrease_liquidity_v2(cpi_context, liquidity, amount_0_min, amount_1_min)
}

pub(super) fn close_clmm_position<'a, 'b, 'c: 'info, 'info>(
    ctx: Context<'a, 'b, 'c, 'info, CloseClmmPosition<'info>>,
) -> Result<()> {
    require!(
        ctx.accounts.vault.strategy_locked,
        VaultError::InvalidStrategyState
    );
    require_keys_eq!(
        ctx.accounts.position_nft_mint.key(),
        ctx.accounts.vault.position_nft_mint,
        VaultError::InvalidClmmPosition
    );
    require!(
        ctx.accounts.personal_position.liquidity == 0
            && ctx.accounts.personal_position.token_fees_owed_0 == 0
            && ctx.accounts.personal_position.token_fees_owed_1 == 0
            && ctx
                .accounts
                .personal_position
                .reward_infos
                .iter()
                .all(|reward| reward.reward_amount_owed == 0),
        VaultError::InvalidClmmPosition
    );
    validate_personal_position(
        &ctx.accounts.personal_position,
        ctx.accounts.pool_state.key(),
        ctx.accounts.position_nft_mint.key(),
    )?;

    let vault_key = ctx.accounts.vault.key();
    let bump = [ctx.accounts.vault.strategy_authority_bump];
    let signer_seeds: &[&[u8]] = &[b"strategy-authority", vault_key.as_ref(), &bump];
    let signer_seeds_set: &[&[&[u8]]] = &[signer_seeds];
    let cpi_accounts = cpi::accounts::ClosePosition {
        nft_owner: ctx.accounts.strategy_authority.to_account_info(),
        position_nft_mint: ctx.accounts.position_nft_mint.to_account_info(),
        position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
        personal_position: ctx.accounts.personal_position.to_account_info(),
        system_program: ctx.accounts.system_program.to_account_info(),
        token_program: ctx.accounts.token_program.to_account_info(),
    };
    let cpi_context = CpiContext::new_with_signer(
        ctx.accounts.clmm_program.to_account_info(),
        cpi_accounts,
        signer_seeds_set,
    )
    .with_remaining_accounts(vec![ctx.accounts.pool_state.to_account_info()]);
    cpi::close_position(cpi_context)?;

    ctx.accounts.vault.position_nft_mint = Pubkey::default();
    ctx.accounts.vault.position_nonce = ctx
        .accounts
        .vault
        .position_nonce
        .checked_add(1)
        .ok_or(VaultError::ArithmeticOverflow)?;
    Ok(())
}

pub(super) fn end_strategy(ctx: Context<EndStrategy>) -> Result<()> {
    require!(
        ctx.accounts.vault.strategy_locked
            && ctx.accounts.vault.position_nft_mint == Pubkey::default()
            && ctx.accounts.quote_token_account.amount == 0,
        VaultError::StrategyNotLiquidated
    );
    ctx.accounts.vault.strategy_locked = false;

    let minimum_balance = Rent::get()?.minimum_balance(0);
    let excess_lamports = ctx
        .accounts
        .strategy_authority
        .lamports()
        .saturating_sub(minimum_balance);
    if excess_lamports > 0 {
        let vault_key = ctx.accounts.vault.key();
        let bump = [ctx.accounts.vault.strategy_authority_bump];
        let signer_seeds: &[&[u8]] = &[b"strategy-authority", vault_key.as_ref(), &bump];
        anchor_lang::solana_program::program::invoke_signed(
            &anchor_lang::solana_program::system_instruction::transfer(
                &ctx.accounts.strategy_authority.key(),
                &ctx.accounts.admin.key(),
                excess_lamports,
            ),
            &[
                ctx.accounts.strategy_authority.to_account_info(),
                ctx.accounts.admin.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            &[signer_seeds],
        )?;
    }
    Ok(())
}

fn validate_pool_mints(vault: &Vault, pool: &PoolState, quote_mint: Pubkey) -> Result<()> {
    require!(
        (pool.token_mint_0 == vault.base_mint && pool.token_mint_1 == quote_mint)
            || (pool.token_mint_1 == vault.base_mint && pool.token_mint_0 == quote_mint),
        VaultError::InvalidStrategyPool
    );
    Ok(())
}

fn validate_configured_pool(vault: &Vault, pool: &PoolState, quote_mint: Pubkey) -> Result<()> {
    require!(
        vault.strategy_pool != Pubkey::default(),
        VaultError::InvalidStrategyState
    );
    validate_pool_mints(vault, pool, quote_mint)
}

fn validate_personal_position(
    position: &PersonalPositionState,
    pool: Pubkey,
    nft_mint: Pubkey,
) -> Result<()> {
    require_keys_eq!(position.pool_id, pool, VaultError::InvalidClmmPosition);
    require_keys_eq!(position.nft_mint, nft_mint, VaultError::InvalidClmmPosition);
    Ok(())
}

fn validate_vault_token_owner(
    base_token_account: &TokenAccount,
    quote_token_account: &TokenAccount,
    strategy_authority: Pubkey,
) -> Result<()> {
    require_keys_eq!(
        base_token_account.owner,
        strategy_authority,
        VaultError::InvalidStrategyTokenAccount
    );
    require_keys_eq!(
        quote_token_account.owner,
        strategy_authority,
        VaultError::InvalidStrategyTokenAccount
    );
    Ok(())
}

fn validate_position_ata(
    address: Pubkey,
    authority: Pubkey,
    mint: Pubkey,
    token_program: Pubkey,
    associated_token_program: Pubkey,
) -> Result<()> {
    let expected = Pubkey::find_program_address(
        &[authority.as_ref(), token_program.as_ref(), mint.as_ref()],
        &associated_token_program,
    )
    .0;
    require_keys_eq!(address, expected, VaultError::InvalidClmmPosition);
    Ok(())
}

fn validate_metadata_address(
    address: Pubkey,
    mint: Pubkey,
    metadata_program: Pubkey,
) -> Result<()> {
    let expected = Pubkey::find_program_address(
        &[b"metadata", metadata_program.as_ref(), mint.as_ref()],
        &metadata_program,
    )
    .0;
    require_keys_eq!(address, expected, VaultError::InvalidClmmPosition);
    Ok(())
}

#[derive(Accounts)]
pub struct ConfigureStrategy<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", base_mint.key().as_ref()],
        bump = vault.bump,
        has_one = admin,
        has_one = base_mint
    )]
    pub vault: Account<'info, Vault>,
    pub base_mint: Account<'info, Mint>,
    pub quote_mint: Account<'info, Mint>,
    pub clmm_program: Program<'info, RaydiumClmm>,
    pub pool_state: AccountLoader<'info, PoolState>,
    #[account(
        mut,
        seeds = [b"strategy-authority", vault.key().as_ref()],
        bump = vault.strategy_authority_bump
    )]
    pub strategy_authority: SystemAccount<'info>,
    #[account(
        init_if_needed,
        payer = admin,
        associated_token::mint = quote_mint,
        associated_token::authority = strategy_authority
    )]
    pub quote_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct BeginStrategy<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", vault.base_mint.as_ref()],
        bump = vault.bump,
        has_one = admin
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        mut,
        seeds = [b"strategy-authority", vault.key().as_ref()],
        bump = vault.strategy_authority_bump
    )]
    pub strategy_authority: SystemAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Swap<'info> {
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", vault.base_mint.as_ref()],
        bump = vault.bump,
        has_one = admin,
        constraint = vault.strategy_locked
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        seeds = [b"strategy-authority", vault.key().as_ref()],
        bump = vault.strategy_authority_bump
    )]
    pub strategy_authority: SystemAccount<'info>,
    #[account(address = vault.base_mint)]
    pub base_mint: Account<'info, Mint>,
    #[account(address = vault.quote_mint)]
    pub quote_mint: Account<'info, Mint>,
    pub clmm_program: Program<'info, RaydiumClmm>,
    #[account(mut, address = vault.strategy_pool)]
    pub pool_state: AccountLoader<'info, PoolState>,
    #[account(address = pool_state.load()?.amm_config)]
    pub amm_config: Box<Account<'info, AmmConfig>>,
    #[account(
        mut,
        associated_token::mint = base_mint,
        associated_token::authority = strategy_authority
    )]
    pub base_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        associated_token::mint = quote_mint,
        associated_token::authority = strategy_authority
    )]
    pub quote_token_account: Account<'info, TokenAccount>,
    #[account(mut, address = pool_state.load()?.token_vault_0)]
    pub pool_vault_0: Account<'info, TokenAccount>,
    #[account(mut, address = pool_state.load()?.token_vault_1)]
    pub pool_vault_1: Account<'info, TokenAccount>,
    #[account(mut, address = pool_state.load()?.observation_key)]
    pub observation_state: AccountLoader<'info, ObservationState>,
    #[account(address = pool_state.load()?.token_mint_0)]
    pub mint_0: Account<'info, Mint>,
    #[account(address = pool_state.load()?.token_mint_1)]
    pub mint_1: Account<'info, Mint>,
    pub token_program: Program<'info, Token>,
    pub token_program_2022: Program<'info, Token2022>,
    pub memo_program: Program<'info, Memo>,
}

#[derive(Accounts)]
#[instruction(tick_lower_index: i32, tick_upper_index: i32, tick_array_lower_start_index: i32, tick_array_upper_start_index: i32)]
pub struct OpenClmmPosition<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", vault.base_mint.as_ref()],
        bump = vault.bump,
        has_one = admin,
        constraint = vault.strategy_locked
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        mut,
        seeds = [b"strategy-authority", vault.key().as_ref()],
        bump = vault.strategy_authority_bump
    )]
    pub strategy_authority: SystemAccount<'info>,
    #[account(address = vault.base_mint)]
    pub base_mint: Account<'info, Mint>,
    #[account(address = vault.quote_mint)]
    pub quote_mint: Account<'info, Mint>,
    pub clmm_program: Program<'info, RaydiumClmm>,
    #[account(mut, address = vault.strategy_pool)]
    pub pool_state: AccountLoader<'info, PoolState>,
    /// CHECK: Mint address is a per-vault PDA created by the Raydium CPI.
    #[account(
        mut,
        seeds = [b"clmm-position-mint", vault.key().as_ref(), &vault.position_nonce.to_le_bytes()],
        bump
    )]
    pub position_nft_mint: UncheckedAccount<'info>,
    /// CHECK: Canonical position NFT ATA; Raydium creates it.
    #[account(mut)]
    pub position_nft_account: UncheckedAccount<'info>,
    /// CHECK: Canonical Metaplex metadata PDA.
    #[account(mut)]
    pub metadata_account: UncheckedAccount<'info>,
    /// CHECK: Deprecated Raydium account retained for CPI compatibility.
    #[account(
        mut,
        seeds = [
            POSITION_SEED.as_bytes(),
            pool_state.key().as_ref(),
            &tick_lower_index.to_be_bytes(),
            &tick_upper_index.to_be_bytes()
        ],
        seeds::program = clmm_program.key(),
        bump
    )]
    pub protocol_position: UncheckedAccount<'info>,
    /// CHECK: Derived Raydium tick array account.
    #[account(
        mut,
        seeds = [
            TICK_ARRAY_SEED.as_bytes(),
            pool_state.key().as_ref(),
            &tick_array_lower_start_index.to_be_bytes()
        ],
        seeds::program = clmm_program.key(),
        bump
    )]
    pub tick_array_lower: UncheckedAccount<'info>,
    /// CHECK: Derived Raydium tick array account.
    #[account(
        mut,
        seeds = [
            TICK_ARRAY_SEED.as_bytes(),
            pool_state.key().as_ref(),
            &tick_array_upper_start_index.to_be_bytes()
        ],
        seeds::program = clmm_program.key(),
        bump
    )]
    pub tick_array_upper: UncheckedAccount<'info>,
    /// CHECK: Raydium creates the personal position PDA.
    #[account(
        mut,
        seeds = [POSITION_SEED.as_bytes(), position_nft_mint.key().as_ref()],
        seeds::program = clmm_program.key(),
        bump
    )]
    pub personal_position: UncheckedAccount<'info>,
    #[account(
        mut,
        associated_token::mint = base_mint,
        associated_token::authority = strategy_authority
    )]
    pub base_token_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        associated_token::mint = quote_mint,
        associated_token::authority = strategy_authority
    )]
    pub quote_token_account: Account<'info, TokenAccount>,
    #[account(mut, address = pool_state.load()?.token_vault_0)]
    pub pool_vault_0: Account<'info, TokenAccount>,
    #[account(mut, address = pool_state.load()?.token_vault_1)]
    pub pool_vault_1: Account<'info, TokenAccount>,
    #[account(address = pool_state.load()?.token_mint_0)]
    pub mint_0: Account<'info, Mint>,
    #[account(address = pool_state.load()?.token_mint_1)]
    pub mint_1: Account<'info, Mint>,
    pub rent: Sysvar<'info, Rent>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub metadata_program: Program<'info, Metadata>,
    pub token_program_2022: Program<'info, Token2022>,
}

#[derive(Accounts)]
#[instruction(tick_array_lower_start_index: i32, tick_array_upper_start_index: i32)]
pub struct DecreaseLiquidity<'info> {
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", vault.base_mint.as_ref()],
        bump = vault.bump,
        has_one = admin,
        constraint = vault.strategy_locked
    )]
    pub vault: Box<Account<'info, Vault>>,
    #[account(
        mut,
        seeds = [b"strategy-authority", vault.key().as_ref()],
        bump = vault.strategy_authority_bump
    )]
    pub strategy_authority: SystemAccount<'info>,
    #[account(address = vault.base_mint)]
    pub base_mint: Box<Account<'info, Mint>>,
    #[account(address = vault.quote_mint)]
    pub quote_mint: Box<Account<'info, Mint>>,
    pub clmm_program: Program<'info, RaydiumClmm>,
    #[account(mut, address = vault.strategy_pool)]
    pub pool_state: AccountLoader<'info, PoolState>,
    #[account(
        mut,
        address = vault.position_nft_mint,
        mint::token_program = token_program
    )]
    pub position_nft_mint: Box<Account<'info, anchor_spl::token::Mint>>,
    #[account(
        mut,
        associated_token::mint = position_nft_mint,
        associated_token::authority = strategy_authority,
        constraint = position_nft_account.amount == 1
    )]
    pub position_nft_account: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        seeds = [POSITION_SEED.as_bytes(), position_nft_mint.key().as_ref()],
        seeds::program = clmm_program.key(),
        bump
    )]
    pub personal_position: Box<Account<'info, PersonalPositionState>>,
    /// CHECK: Deprecated Raydium account retained for CPI compatibility.
    #[account(
        mut,
        seeds = [
            POSITION_SEED.as_bytes(),
            pool_state.key().as_ref(),
            &personal_position.tick_lower_index.to_be_bytes(),
            &personal_position.tick_upper_index.to_be_bytes()
        ],
        seeds::program = clmm_program.key(),
        bump
    )]
    pub protocol_position: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [
            TICK_ARRAY_SEED.as_bytes(),
            pool_state.key().as_ref(),
            &tick_array_lower_start_index.to_be_bytes()
        ],
        seeds::program = clmm_program.key(),
        bump
    )]
    pub tick_array_lower: AccountLoader<'info, TickArrayState>,
    #[account(
        mut,
        seeds = [
            TICK_ARRAY_SEED.as_bytes(),
            pool_state.key().as_ref(),
            &tick_array_upper_start_index.to_be_bytes()
        ],
        seeds::program = clmm_program.key(),
        bump
    )]
    pub tick_array_upper: AccountLoader<'info, TickArrayState>,
    #[account(mut, address = pool_state.load()?.token_vault_0)]
    pub pool_vault_0: Box<Account<'info, TokenAccount>>,
    #[account(mut, address = pool_state.load()?.token_vault_1)]
    pub pool_vault_1: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        associated_token::mint = base_mint,
        associated_token::authority = strategy_authority
    )]
    pub base_token_account: Box<Account<'info, TokenAccount>>,
    #[account(
        mut,
        associated_token::mint = quote_mint,
        associated_token::authority = strategy_authority
    )]
    pub quote_token_account: Box<Account<'info, TokenAccount>>,
    #[account(address = pool_state.load()?.token_mint_0)]
    pub mint_0: Box<Account<'info, Mint>>,
    #[account(address = pool_state.load()?.token_mint_1)]
    pub mint_1: Box<Account<'info, Mint>>,
    pub token_program: Program<'info, Token>,
    pub token_program_2022: Program<'info, Token2022>,
    pub memo_program: Program<'info, Memo>,
}

#[derive(Accounts)]
pub struct CloseClmmPosition<'info> {
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", vault.base_mint.as_ref()],
        bump = vault.bump,
        has_one = admin,
        constraint = vault.strategy_locked
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        mut,
        seeds = [b"strategy-authority", vault.key().as_ref()],
        bump = vault.strategy_authority_bump
    )]
    pub strategy_authority: SystemAccount<'info>,
    pub clmm_program: Program<'info, RaydiumClmm>,
    #[account(address = vault.strategy_pool)]
    pub pool_state: AccountLoader<'info, PoolState>,
    #[account(address = vault.position_nft_mint)]
    pub position_nft_mint: Account<'info, anchor_spl::token::Mint>,
    #[account(
        mut,
        associated_token::mint = position_nft_mint,
        associated_token::authority = strategy_authority,
        constraint = position_nft_account.amount == 1
    )]
    pub position_nft_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        seeds = [POSITION_SEED.as_bytes(), position_nft_mint.key().as_ref()],
        seeds::program = clmm_program.key(),
        bump
    )]
    pub personal_position: Account<'info, PersonalPositionState>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct EndStrategy<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [b"vault", vault.base_mint.as_ref()],
        bump = vault.bump,
        has_one = admin
    )]
    pub vault: Account<'info, Vault>,
    #[account(
        mut,
        seeds = [b"strategy-authority", vault.key().as_ref()],
        bump = vault.strategy_authority_bump
    )]
    pub strategy_authority: SystemAccount<'info>,
    #[account(address = vault.quote_mint)]
    pub quote_mint: Account<'info, Mint>,
    #[account(
        associated_token::mint = quote_mint,
        associated_token::authority = strategy_authority
    )]
    pub quote_token_account: Account<'info, TokenAccount>,
    pub system_program: Program<'info, System>,
}
