import {
  TOKEN_PROGRAM_ID,
  createMint,
  getOrCreateAssociatedTokenAccount,
  mintTo,
} from "@solana/spl-token";
import { PublicKey } from "@solana/web3.js";
import { connection, explorerUrl, loadLocalSigner } from "./wallet.mjs";

const decimals = 6;
const supply = 1_000_000_000n;
const payer = loadLocalSigner();
const recipient = process.env.TOKEN_RECIPIENT
  ? new PublicKey(process.env.TOKEN_RECIPIENT)
  : payer.publicKey;

async function createTestMint(label) {
  const mint = await createMint(
    connection,
    payer,
    payer.publicKey,
    null,
    decimals,
    undefined,
    undefined,
    TOKEN_PROGRAM_ID,
  );
  const tokenAccount = await getOrCreateAssociatedTokenAccount(
    connection,
    payer,
    mint,
    recipient,
  );
  const signature = await mintTo(
    connection,
    payer,
    mint,
    tokenAccount.address,
    payer,
    supply,
  );
  return {
    label,
    mint: mint.toBase58(),
    decimals,
    recipient: recipient.toBase58(),
    ownerTokenAccount: tokenAccount.address.toBase58(),
    mintedTokens: "1000",
    mintTransaction: explorerUrl(signature),
  };
}

const base = await createTestMint("BASE");
console.log("Created test mint:", JSON.stringify(base, null, 2));
const quote = await createTestMint("QUOTE");
console.log("Created test mint:", JSON.stringify(quote, null, 2));
