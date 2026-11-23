import Decimal from "decimal.js";
import { Connection, PublicKey } from "@solana/web3.js";
import {
  DEVNET_PROGRAM_ID,
  DEV_API_URLS,
  Raydium,
  TxVersion,
} from "@raydium-io/raydium-sdk-v2";
import { connection, explorerUrl, loadLocalSigner } from "./wallet.mjs";

const baseMintAddress = process.env.BASE_MINT;
const quoteMintAddress = process.env.QUOTE_MINT;
const initialPriceValue = process.env.INITIAL_PRICE;
if (!baseMintAddress || !quoteMintAddress || !initialPriceValue) {
  throw new Error(
    "Set BASE_MINT, QUOTE_MINT and INITIAL_PRICE (quote tokens per base token).",
  );
}
const initialPrice = new Decimal(initialPriceValue);
if (!initialPrice.isFinite() || !initialPrice.isPositive()) {
  throw new Error("INITIAL_PRICE must be a positive finite number.");
}
const baseMint = new PublicKey(baseMintAddress);
const quoteMint = new PublicKey(quoteMintAddress);
if (baseMint.equals(quoteMint)) {
  throw new Error("BASE_MINT and QUOTE_MINT must be different addresses.");
}

const owner = loadLocalSigner();
const raydium = await Raydium.load({
  owner,
  connection: new Connection(connection.rpcEndpoint, "confirmed"),
  cluster: "devnet",
  disableFeatureCheck: true,
  disableLoadToken: false,
  blockhashCommitment: "confirmed",
  urlConfigs: {
    ...DEV_API_URLS,
    BASE_HOST: "https://api-v3-devnet.raydium.io",
    OWNER_BASE_HOST: "https://owner-v1-devnet.raydium.io",
    SWAP_HOST: "https://transaction-v1-devnet.raydium.io",
    CPMM_LOCK: "https://dynamic-ipfs-devnet.raydium.io/lock/cpmm/position",
  },
});

const [mint1, mint2, configs] = await Promise.all([
  raydium.token.getTokenInfo(baseMint),
  raydium.token.getTokenInfo(quoteMint),
  raydium.api.getClmmConfigs(),
]);
if (configs.length === 0) {
  throw new Error("Raydium Devnet API returned no CLMM configurations.");
}

const { execute, extInfo } = await raydium.clmm.createPool({
  programId: DEVNET_PROGRAM_ID.CLMM_PROGRAM_ID,
  mint1,
  mint2,
  ammConfig: {
    ...configs[0],
    id: new PublicKey(configs[0].id),
    fundOwner: "",
    description: "",
  },
  initialPrice,
  txVersion: TxVersion.V0,
});
const { txId } = await execute({ sendAndConfirm: true });

console.log(
  "Created Raydium CLMM Devnet pool:",
  JSON.stringify(
    {
      poolId: extInfo.address.id,
      programId: extInfo.address.programId,
      ammConfig: extInfo.address.config.id,
      baseMint: baseMint.toBase58(),
      quoteMint: quoteMint.toBase58(),
      mintA: extInfo.address.mintA.address,
      mintB: extInfo.address.mintB.address,
      poolVaultA: extInfo.address.vault.A,
      poolVaultB: extInfo.address.vault.B,
      observationState: extInfo.address.observationId,
      exBitmapAccount: extInfo.address.exBitmapAccount,
      tickSpacing: extInfo.address.config.tickSpacing,
      initialPriceQuotePerBase: initialPrice.toString(),
      transaction: explorerUrl(txId),
    },
    null,
    2,
  ),
);
