import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { Connection, Keypair } from "@solana/web3.js";

export const DEVNET_RPC =
  process.env.SOLANA_RPC_URL ?? "https://api.devnet.solana.com";

if (!/devnet/i.test(DEVNET_RPC)) {
  throw new Error(
    "SOLANA_RPC_URL must identify a Devnet endpoint; refusing to run against another cluster.",
  );
}

export const connection = new Connection(DEVNET_RPC, "confirmed");

export function loadLocalSigner() {
  const walletPath = process.env.ANCHOR_WALLET;
  if (!walletPath) {
    throw new Error(
      "Set ANCHOR_WALLET to your local Solana keypair path. Never share or commit that file.",
    );
  }
  const secret = JSON.parse(readFileSync(resolve(walletPath), "utf8"));
  if (
    !Array.isArray(secret) ||
    secret.some((value) => !Number.isInteger(value) || value < 0 || value > 255)
  ) {
    throw new Error(
      "ANCHOR_WALLET must point to a Solana keypair JSON byte array.",
    );
  }
  return Keypair.fromSecretKey(Uint8Array.from(secret));
}

export function explorerUrl(signature) {
  return `https://explorer.solana.com/tx/${signature}?cluster=devnet`;
}
