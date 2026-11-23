import * as anchor from "@coral-xyz/anchor";
import { assert } from "chai";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  createMint,
  getAccount,
  getAssociatedTokenAddressSync,
  getOrCreateAssociatedTokenAccount,
  mintTo,
} from "@solana/spl-token";
import { LiquidityVault } from "../idl/liquidity_vault";
import idl from "../idl/liquidity_vault.json";

describe("liquidity_vault", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = new anchor.Program<LiquidityVault>(
    idl as LiquidityVault,
    provider,
  );
  const admin = provider.wallet.publicKey;
  const user = anchor.web3.Keypair.generate();
  const user2 = anchor.web3.Keypair.generate();
  let baseMint: anchor.web3.PublicKey;
  let vault: anchor.web3.PublicKey;
  let shareMint: anchor.web3.PublicKey;
  let strategyAuthority: anchor.web3.PublicKey;
  let vaultTokenAccount: anchor.web3.PublicKey;
  let userBaseAccount: anchor.web3.PublicKey;
  let userShareAccount: anchor.web3.PublicKey;
  let user2BaseAccount: anchor.web3.PublicKey;
  let user2ShareAccount: anchor.web3.PublicKey;
  let position: anchor.web3.PublicKey;
  let position2: anchor.web3.PublicKey;

  it("initializes, deposits proportionally, and withdraws by burning shares", async () => {
    const payer = anchor.Wallet.local().payer;
    for (const wallet of [user, user2]) {
      const signature = await provider.connection.requestAirdrop(
        wallet.publicKey,
        anchor.web3.LAMPORTS_PER_SOL,
      );
      const latest = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction(
        { signature, ...latest },
        "confirmed",
      );
    }

    baseMint = await createMint(provider.connection, payer, admin, admin, 0);
    [vault] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), baseMint.toBuffer()],
      program.programId,
    );
    [shareMint] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("share"), vault.toBuffer()],
      program.programId,
    );
    [strategyAuthority] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("strategy-authority"), vault.toBuffer()],
      program.programId,
    );
    [position] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("position"), vault.toBuffer(), user.publicKey.toBuffer()],
      program.programId,
    );
    [position2] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("position"), vault.toBuffer(), user2.publicKey.toBuffer()],
      program.programId,
    );
    vaultTokenAccount = getAssociatedTokenAddressSync(
      baseMint,
      strategyAuthority,
      true,
    );
    userBaseAccount = (
      await getOrCreateAssociatedTokenAccount(
        provider.connection,
        payer,
        baseMint,
        user.publicKey,
      )
    ).address;
    userShareAccount = getAssociatedTokenAddressSync(shareMint, user.publicKey);
    user2BaseAccount = (
      await getOrCreateAssociatedTokenAccount(
        provider.connection,
        payer,
        baseMint,
        user2.publicKey,
      )
    ).address;
    user2ShareAccount = getAssociatedTokenAddressSync(
      shareMint,
      user2.publicKey,
    );

    await program.methods
      .initializeVault()
      .accounts({
        admin,
        baseMint,
        vault,
        shareMint,
        strategyAuthority,
        vaultTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .rpc();

    for (const [wallet, userPosition] of [
      [user, position],
      [user2, position2],
    ] as const) {
      await program.methods
        .openPosition()
        .accounts({
          user: wallet.publicKey,
          vault,
          position: userPosition,
          systemProgram: anchor.web3.SystemProgram.programId,
        })
        .signers([wallet])
        .rpc();
    }

    await mintTo(
      provider.connection,
      payer,
      baseMint,
      userBaseAccount,
      admin,
      1_000,
    );
    await mintTo(
      provider.connection,
      payer,
      baseMint,
      user2BaseAccount,
      admin,
      110,
    );

    const depositAccounts = (
      wallet: anchor.web3.Keypair,
      baseAccount: anchor.web3.PublicKey,
      shareAccount: anchor.web3.PublicKey,
      userPosition: anchor.web3.PublicKey,
    ) => ({
      user: wallet.publicKey,
      vault,
      strategyAuthority,
      baseMint,
      shareMint,
      vaultTokenAccount,
      userBaseAccount: baseAccount,
      userShareAccount: shareAccount,
      position: userPosition,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: anchor.web3.SystemProgram.programId,
    });

    await program.methods
      .deposit(new anchor.BN(1_000))
      .accounts(
        depositAccounts(user, userBaseAccount, userShareAccount, position),
      )
      .signers([user])
      .rpc();
    assert.equal(
      Number((await getAccount(provider.connection, userShareAccount)).amount),
      1_000,
    );

    await mintTo(
      provider.connection,
      payer,
      baseMint,
      vaultTokenAccount,
      admin,
      100,
    );
    await program.methods
      .deposit(new anchor.BN(110))
      .accounts(
        depositAccounts(user2, user2BaseAccount, user2ShareAccount, position2),
      )
      .signers([user2])
      .rpc();
    assert.equal(
      Number((await getAccount(provider.connection, user2ShareAccount)).amount),
      100,
    );

    await program.methods
      .withdraw(new anchor.BN(1_000))
      .accounts({
        user: user.publicKey,
        vault,
        strategyAuthority,
        baseMint,
        shareMint,
        vaultTokenAccount,
        userBaseAccount,
        userShareAccount,
        position,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([user])
      .rpc();
    assert.equal(
      Number((await getAccount(provider.connection, userBaseAccount)).amount),
      1_100,
    );
    assert.equal(
      Number((await getAccount(provider.connection, userShareAccount)).amount),
      0,
    );
  });
});
