import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { Keypair } from "@solana/web3.js";

const keypairPath = resolve("target", "deploy", "liquidity_vault-keypair.json");
const idlPath = resolve("idl", "liquidity_vault.json");
const secretKey = JSON.parse(readFileSync(keypairPath, "utf8"));

if (
  !Array.isArray(secretKey) ||
  secretKey.length !== 64 ||
  secretKey.some((byte) => !Number.isInteger(byte) || byte < 0 || byte > 255)
) {
  throw new Error(`Invalid program keypair file: ${keypairPath}`);
}

const address = Keypair.fromSecretKey(Uint8Array.from(secretKey)).publicKey.toBase58();
const idl = JSON.parse(readFileSync(idlPath, "utf8"));
if (typeof idl.address !== "string") {
  throw new Error(`IDL has no program address: ${idlPath}`);
}

idl.address = address;
writeFileSync(idlPath, `${JSON.stringify(idl, null, 2)}\n`);
console.log(`Updated IDL program address: ${address}`);
