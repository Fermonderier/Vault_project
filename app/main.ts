import "./polyfills";
import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountIdempotentInstruction,
  getAccount,
  getAssociatedTokenAddressSync,
  getMint,
} from "@solana/spl-token";
import {
  Connection,
  PublicKey,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import idl from "../idl/liquidity_vault.json";
import type { LiquidityVault } from "../idl/liquidity_vault";
import "./styles.css";

const DEVNET_RPC = "https://api.devnet.solana.com";
const RAYDIUM_CLMM_DEVNET = new PublicKey(
  "DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH",
);
const METADATA_PROGRAM = new PublicKey(
  "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
);
const typedIdl = idl as LiquidityVault;
const programId = new PublicKey(typedIdl.address);
const connection = new Connection(DEVNET_RPC, "confirmed");

type WalletTransaction =
  anchor.web3.Transaction | anchor.web3.VersionedTransaction;

interface InjectedSolanaProvider {
  publicKey: PublicKey | null;
  connect(): Promise<{ publicKey: PublicKey }>;
  disconnect(): Promise<void>;
  signTransaction<T extends WalletTransaction>(transaction: T): Promise<T>;
  signAllTransactions<T extends WalletTransaction>(
    transactions: T[],
  ): Promise<T[]>;
}

declare global {
  interface Window {
    solana?: InjectedSolanaProvider;
    phantom?: { solana?: InjectedSolanaProvider };
  }
}

interface VaultAddresses {
  baseMint: PublicKey;
  vault: PublicKey;
  shareMint: PublicKey;
  strategyAuthority: PublicKey;
  vaultTokenAccount: PublicKey;
}

interface ParsedAccount {
  [key: string]: unknown;
}

let injectedWallet: InjectedSolanaProvider | undefined;
let provider: anchor.AnchorProvider | undefined;
let program: anchor.Program<LiquidityVault> | undefined;
let vaultAdmin: PublicKey | undefined;

function input(id: string): HTMLInputElement {
  const element = document.getElementById(id);
  if (!(element instanceof HTMLInputElement)) {
    throw new Error(`Missing input element: ${id}`);
  }
  return element;
}

function textarea(id: string): HTMLTextAreaElement {
  const element = document.getElementById(id);
  if (!(element instanceof HTMLTextAreaElement)) {
    throw new Error(`Missing text area: ${id}`);
  }
  return element;
}

function button(id: string): HTMLButtonElement {
  const element = document.getElementById(id);
  if (!(element instanceof HTMLButtonElement)) {
    throw new Error(`Missing button: ${id}`);
  }
  return element;
}

function setText(id: string, value: string): void {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`Missing element: ${id}`);
  }
  element.textContent = value;
}

function showActivity(
  message: string,
  state: "idle" | "pending" | "success" | "error" = "idle",
  signature?: string,
): void {
  setText("activity-message", message);
  setText(
    "activity-state",
    state === "pending"
      ? "Ожидание"
      : state === "success"
        ? "Успешно"
        : state === "error"
          ? "Ошибка"
          : "Готово",
  );
  const stateElement = document.getElementById("activity-state");
  if (stateElement) {
    stateElement.className = `activity-state activity-state-${state}`;
  }
  const link = document.getElementById("transaction-link");
  if (!(link instanceof HTMLAnchorElement)) {
    throw new Error("Missing transaction link element");
  }
  if (signature) {
    link.href = `https://explorer.solana.com/tx/${signature}?cluster=devnet`;
    link.hidden = false;
  } else {
    link.removeAttribute("href");
    link.hidden = true;
  }
}

function publicKey(value: string, label: string): PublicKey {
  try {
    return new PublicKey(value.trim());
  } catch {
    throw new Error(`${label}: укажите корректный адрес Solana.`);
  }
}

function currentWallet(): PublicKey {
  const key = injectedWallet?.publicKey;
  if (!key || !provider || !program) {
    throw new Error("Сначала подключите кошелёк Phantom.");
  }
  return key;
}

function currentProgram(): anchor.Program<LiquidityVault> {
  if (!program || !provider) {
    throw new Error("Сначала подключите кошелёк Phantom.");
  }
  return program;
}

function currentProvider(): anchor.AnchorProvider {
  if (!provider) {
    throw new Error("Сначала подключите кошелёк Phantom.");
  }
  return provider;
}

function deriveAddresses(baseMint: PublicKey): VaultAddresses {
  const [vault] = PublicKey.findProgramAddressSync(
    [new TextEncoder().encode("vault"), baseMint.toBytes()],
    programId,
  );
  const [shareMint] = PublicKey.findProgramAddressSync(
    [new TextEncoder().encode("share"), vault.toBytes()],
    programId,
  );
  const [strategyAuthority] = PublicKey.findProgramAddressSync(
    [new TextEncoder().encode("strategy-authority"), vault.toBytes()],
    programId,
  );
  return {
    baseMint,
    vault,
    shareMint,
    strategyAuthority,
    vaultTokenAccount: getAssociatedTokenAddressSync(
      baseMint,
      strategyAuthority,
      true,
    ),
  };
}

function formatAmount(amount: bigint, decimals: number): string {
  if (decimals === 0) {
    return amount.toString();
  }
  const scale = 10n ** BigInt(decimals);
  const whole = amount / scale;
  const fractional = (amount % scale)
    .toString()
    .padStart(decimals, "0")
    .replace(/0+$/, "");
  return fractional ? `${whole}.${fractional}` : whole.toString();
}

function toBaseUnits(
  value: string,
  decimals: number,
  label: string,
): anchor.BN {
  const normalized = value.trim();
  if (!/^\d+(\.\d+)?$/.test(normalized)) {
    throw new Error(`${label}: введите положительное число.`);
  }
  const [whole, fraction = ""] = normalized.split(".");
  if (fraction.length > decimals) {
    throw new Error(`${label}: максимум ${decimals} знаков после запятой.`);
  }
  const paddedFraction = fraction.padEnd(decimals, "0");
  const scale = 10n ** BigInt(decimals);
  const raw =
    BigInt(whole) * scale +
    BigInt(paddedFraction.length === 0 ? "0" : paddedFraction);
  if (raw <= 0n) {
    throw new Error(`${label}: сумма должна быть больше нуля.`);
  }
  return new anchor.BN(raw.toString());
}

function asRecord(value: unknown, label: string): ParsedAccount {
  if (!isRecord(value)) {
    throw new Error(`${label}: нужен JSON-объект.`);
  }
  return value;
}

function parseJsonObject(id: string, label: string): ParsedAccount {
  try {
    const parsed: unknown = JSON.parse(textarea(id).value);
    return asRecord(parsed, label);
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new Error(`${label}: некорректный JSON.`);
    }
    throw error;
  }
}

function getAddresses(): VaultAddresses {
  const baseMint = publicKey(input("base-mint").value, "Base mint");
  return deriveAddresses(baseMint);
}

function showAddress(label: string, key: PublicKey): HTMLDivElement {
  const row = document.createElement("div");
  row.className = "address-row";
  const name = document.createElement("span");
  name.textContent = label;
  const link = document.createElement("a");
  link.href = `https://explorer.solana.com/address/${key.toBase58()}?cluster=devnet`;
  link.target = "_blank";
  link.rel = "noreferrer";
  link.textContent = key.toBase58();
  row.append(name, link);
  return row;
}

async function getTokenAmount(address: PublicKey): Promise<bigint> {
  const accountInfo = await connection.getAccountInfo(address, "confirmed");
  if (!accountInfo) {
    return 0n;
  }
  return (await getAccount(connection, address, "confirmed")).amount;
}

async function refreshVault(): Promise<void> {
  const owner = currentWallet();
  const current = currentProgram();
  const addresses = getAddresses();
  const [baseMintInfo, programInfo] = await Promise.all([
    getMint(connection, addresses.baseMint),
    connection.getAccountInfo(programId, "confirmed"),
  ]);
  const decimals = baseMintInfo.decimals;
  const userBase = getAssociatedTokenAddressSync(addresses.baseMint, owner);
  const userBaseAmount = await getTokenAmount(userBase);
  const vaultInfo = await connection.getAccountInfo(
    addresses.vault,
    "confirmed",
  );
  const addressList = document.getElementById("address-list");
  if (!addressList) {
    throw new Error("Missing address list");
  }
  addressList.replaceChildren(
    showAddress("Vault PDA", addresses.vault),
    showAddress("Share mint", addresses.shareMint),
    showAddress("Strategy PDA", addresses.strategyAuthority),
    showAddress("Vault token account", addresses.vaultTokenAccount),
  );
  setText("vault-balance-symbol", "base token");
  setText("user-balance-symbol", "base / shares");

  if (!programInfo?.executable) {
    setText("vault-balance", "—");
    setText("share-supply", "—");
    setText("user-balances", `${formatAmount(userBaseAmount, decimals)} / —`);
    setText("strategy-status", "Программа не развёрнута");
    setText("strategy-pool", "Сначала нужен Devnet deploy");
    vaultAdmin = undefined;
    const adminPanel = document.getElementById("admin-panel");
    if (adminPanel) {
      adminPanel.hidden = true;
    }
    button("initialize-vault").hidden = false;
    button("initialize-vault").disabled = true;
    button("open-position").hidden = true;
    button("deposit").disabled = true;
    button("withdraw").disabled = true;
    button("configure-strategy").disabled = true;
    button("begin-strategy").disabled = true;
    button("end-strategy").disabled = true;
    button("execute-instruction").disabled = true;
    button("fill-accounts").disabled = true;
    showActivity(
      `Программа ${programId.toBase58()} не найдена в Devnet. Депозит и создание vault пока недоступны.`,
      "error",
    );
    return;
  }

  if (!vaultInfo) {
    setText("vault-balance", "0");
    setText("share-supply", "0");
    setText("user-balances", `${formatAmount(userBaseAmount, decimals)} / 0`);
    setText("strategy-status", "Не создан");
    setText("strategy-pool", "vault ещё не инициализирован");
    vaultAdmin = undefined;
    const adminPanel = document.getElementById("admin-panel");
    if (adminPanel) {
      adminPanel.hidden = true;
    }
    button("initialize-vault").hidden = false;
    button("initialize-vault").disabled = false;
    button("open-position").hidden = true;
    button("deposit").disabled = true;
    button("withdraw").disabled = true;
    button("configure-strategy").disabled = true;
    button("begin-strategy").disabled = true;
    button("end-strategy").disabled = true;
    button("execute-instruction").disabled = true;
    button("fill-accounts").disabled = true;
    showActivity("Vault по этому mint ещё не создан.");
    return;
  }

  const state = await current.account.vault.fetch(addresses.vault);
  const userShares = getAssociatedTokenAddressSync(addresses.shareMint, owner);
  const [userPosition] = PublicKey.findProgramAddressSync(
    [
      new TextEncoder().encode("position"),
      addresses.vault.toBytes(),
      owner.toBytes(),
    ],
    programId,
  );
  const [vaultAmount, shareSupply, userShareAmount] = await Promise.all([
    getTokenAmount(addresses.vaultTokenAccount),
    getMint(connection, addresses.shareMint).then((mint) => mint.supply),
    getTokenAmount(userShares),
  ]);

  vaultAdmin = state.admin;
  button("initialize-vault").hidden = true;
  button("open-position").hidden = Boolean(
    await connection.getAccountInfo(userPosition, "confirmed"),
  );
  button("open-position").disabled = false;
  setText("vault-balance", formatAmount(vaultAmount, decimals));
  setText("share-supply", formatAmount(shareSupply, decimals));
  setText(
    "user-balances",
    `${formatAmount(userBaseAmount, decimals)} / ${formatAmount(userShareAmount, decimals)}`,
  );
  setText("strategy-status", state.strategyLocked ? "Заблокирован" : "Открыт");
  setText(
    "strategy-pool",
    state.strategyPool.equals(PublicKey.default)
      ? "pool не настроен"
      : state.strategyPool.toBase58(),
  );

  const isAdmin = state.admin.equals(owner);
  const adminPanel = document.getElementById("admin-panel");
  if (adminPanel) {
    adminPanel.hidden = !isAdmin;
  }
  button("configure-strategy").disabled =
    !isAdmin ||
    state.strategyLocked ||
    !state.strategyPool.equals(PublicKey.default);
  button("begin-strategy").disabled =
    !isAdmin ||
    state.strategyLocked ||
    state.strategyPool.equals(PublicKey.default);
  button("end-strategy").disabled = !isAdmin || !state.strategyLocked;
  button("deposit").disabled = state.strategyLocked;
  button("withdraw").disabled = state.strategyLocked;
  button("execute-instruction").disabled = !isAdmin;
  button("fill-accounts").disabled = !isAdmin;
  showActivity(
    isAdmin ? "Подключён кошелёк администратора." : "Данные vault обновлены.",
  );
}

async function runAction(action: () => Promise<void>): Promise<void> {
  try {
    showActivity("Подтвердите действие в Phantom…", "pending");
    await action();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    showActivity(message, "error");
  }
}

async function recordTransaction(
  label: string,
  action: Promise<string>,
): Promise<void> {
  const signature = await action;
  showActivity(`${label} подтверждена.`, "success", signature);
  await refreshVault();
}

async function connectWallet(): Promise<void> {
  const candidate = window.phantom?.solana ?? window.solana;
  if (!candidate) {
    throw new Error("Phantom не найден. Установите Phantom и выберите Devnet.");
  }
  injectedWallet = candidate;
  const { publicKey: owner } = await candidate.connect();
  const wallet = {
    get publicKey(): PublicKey {
      const key = candidate.publicKey;
      if (!key) {
        throw new Error("Кошелёк отключён.");
      }
      return key;
    },
    signTransaction: <T extends WalletTransaction>(transaction: T) =>
      candidate.signTransaction(transaction),
    signAllTransactions: <T extends WalletTransaction>(transactions: T[]) =>
      candidate.signAllTransactions(transactions),
  };
  provider = new anchor.AnchorProvider(
    connection,
    wallet,
    anchor.AnchorProvider.defaultOptions(),
  );
  program = new anchor.Program<LiquidityVault>(typedIdl, provider);
  setText(
    "wallet-label",
    `${owner.toBase58().slice(0, 4)}…${owner.toBase58().slice(-4)}`,
  );
  button("connect-wallet").hidden = true;
  button("disconnect-wallet").hidden = false;
  button("refresh-vault").disabled = false;
  button("initialize-vault").disabled = false;
  button("deposit").disabled = false;
  button("withdraw").disabled = false;
  button("execute-instruction").disabled = false;
  button("fill-accounts").disabled = false;
  await refreshVault();
}

async function ensurePosition(addresses: VaultAddresses): Promise<void> {
  const owner = currentWallet();
  const current = currentProgram();
  const [position] = PublicKey.findProgramAddressSync(
    [
      new TextEncoder().encode("position"),
      addresses.vault.toBytes(),
      owner.toBytes(),
    ],
    programId,
  );
  if (await connection.getAccountInfo(position, "confirmed")) {
    return;
  }
  await recordTransaction(
    "Профиль пользователя",
    current.methods
      .openPosition()
      .accounts({
        user: owner,
        vault: addresses.vault,
        position,
        systemProgram: SystemProgram.programId,
      })
      .rpc(),
  );
}

async function ensureBaseTokenAccount(
  mint: PublicKey,
  owner: PublicKey,
): Promise<PublicKey> {
  const tokenAccount = getAssociatedTokenAddressSync(mint, owner);
  if (!(await connection.getAccountInfo(tokenAccount, "confirmed"))) {
    const instruction = createAssociatedTokenAccountIdempotentInstruction(
      owner,
      tokenAccount,
      owner,
      mint,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const signature = await currentProvider().sendAndConfirm(
      new Transaction().add(instruction),
      [],
    );
    showActivity("Создан token account пользователя.", "success", signature);
  }
  return tokenAccount;
}

async function initializeVault(): Promise<void> {
  const owner = currentWallet();
  const current = currentProgram();
  const addresses = getAddresses();
  await recordTransaction(
    "Создание vault",
    current.methods
      .initializeVault()
      .accounts({
        admin: owner,
        baseMint: addresses.baseMint,
        vault: addresses.vault,
        shareMint: addresses.shareMint,
        strategyAuthority: addresses.strategyAuthority,
        vaultTokenAccount: addresses.vaultTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc(),
  );
}

async function createUserPosition(): Promise<void> {
  const addresses = getAddresses();
  await ensurePosition(addresses);
  await refreshVault();
}

async function deposit(): Promise<void> {
  const owner = currentWallet();
  const current = currentProgram();
  const addresses = getAddresses();
  const decimals = (await getMint(connection, addresses.baseMint)).decimals;
  const amount = toBaseUnits(
    input("deposit-amount").value,
    decimals,
    "Депозит",
  );
  await ensurePosition(addresses);
  const userBaseAccount = await ensureBaseTokenAccount(
    addresses.baseMint,
    owner,
  );
  const userShareAccount = getAssociatedTokenAddressSync(
    addresses.shareMint,
    owner,
  );
  const [position] = PublicKey.findProgramAddressSync(
    [
      new TextEncoder().encode("position"),
      addresses.vault.toBytes(),
      owner.toBytes(),
    ],
    programId,
  );
  await recordTransaction(
    "Депозит",
    current.methods
      .deposit(amount)
      .accounts({
        user: owner,
        vault: addresses.vault,
        strategyAuthority: addresses.strategyAuthority,
        baseMint: addresses.baseMint,
        shareMint: addresses.shareMint,
        vaultTokenAccount: addresses.vaultTokenAccount,
        userBaseAccount,
        userShareAccount,
        position,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc(),
  );
}

async function withdraw(): Promise<void> {
  const owner = currentWallet();
  const current = currentProgram();
  const addresses = getAddresses();
  const decimals = (await getMint(connection, addresses.baseMint)).decimals;
  const shares = toBaseUnits(
    input("withdraw-amount").value,
    decimals,
    "Вывод shares",
  );
  const userBaseAccount = await ensureBaseTokenAccount(
    addresses.baseMint,
    owner,
  );
  const userShareAccount = getAssociatedTokenAddressSync(
    addresses.shareMint,
    owner,
  );
  const [position] = PublicKey.findProgramAddressSync(
    [
      new TextEncoder().encode("position"),
      addresses.vault.toBytes(),
      owner.toBytes(),
    ],
    programId,
  );
  await recordTransaction(
    "Вывод",
    current.methods
      .withdraw(shares)
      .accounts({
        user: owner,
        vault: addresses.vault,
        strategyAuthority: addresses.strategyAuthority,
        baseMint: addresses.baseMint,
        shareMint: addresses.shareMint,
        vaultTokenAccount: addresses.vaultTokenAccount,
        userBaseAccount,
        userShareAccount,
        position,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc(),
  );
}

async function configureStrategy(): Promise<void> {
  const owner = currentWallet();
  const current = currentProgram();
  const addresses = getAddresses();
  const quoteMint = publicKey(input("quote-mint").value, "Quote mint");
  const poolState = publicKey(input("pool-state").value, "Raydium pool");
  const quoteTokenAccount = getAssociatedTokenAddressSync(
    quoteMint,
    addresses.strategyAuthority,
    true,
  );
  await recordTransaction(
    "Настройка стратегии",
    current.methods
      .configureStrategy()
      .accounts({
        admin: owner,
        vault: addresses.vault,
        baseMint: addresses.baseMint,
        quoteMint,
        clmmProgram: RAYDIUM_CLMM_DEVNET,
        poolState,
        strategyAuthority: addresses.strategyAuthority,
        quoteTokenAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc(),
  );
}

async function beginStrategy(): Promise<void> {
  const owner = currentWallet();
  const current = currentProgram();
  const addresses = getAddresses();
  const funding = input("funding-lamports").value.trim();
  if (!/^\d+$/.test(funding)) {
    throw new Error(
      "Пополнение strategy PDA должно быть целым числом lamports.",
    );
  }
  await recordTransaction(
    "Запуск стратегии",
    current.methods
      .beginStrategy(new anchor.BN(funding))
      .accounts({
        admin: owner,
        vault: addresses.vault,
        strategyAuthority: addresses.strategyAuthority,
        systemProgram: SystemProgram.programId,
      })
      .rpc(),
  );
}

async function endStrategy(): Promise<void> {
  const owner = currentWallet();
  const current = currentProgram();
  const addresses = getAddresses();
  const quoteMint = publicKey(input("quote-mint").value, "Quote mint");
  const quoteTokenAccount = getAssociatedTokenAddressSync(
    quoteMint,
    addresses.strategyAuthority,
    true,
  );
  await recordTransaction(
    "Завершение стратегии",
    current.methods
      .endStrategy()
      .accounts({
        admin: owner,
        vault: addresses.vault,
        strategyAuthority: addresses.strategyAuthority,
        quoteMint,
        quoteTokenAccount,
        systemProgram: SystemProgram.programId,
      })
      .rpc(),
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function encodeArguments(
  instruction: anchor.Idl["instructions"][number],
  values: ParsedAccount,
): ParsedAccount {
  const result: ParsedAccount = {};
  for (const arg of instruction.args) {
    if (!(arg.name in values)) {
      throw new Error(`Не задан аргумент ${arg.name}.`);
    }
    const value = values[arg.name];
    if (
      arg.type === "u64" ||
      arg.type === "u128" ||
      arg.type === "i64" ||
      arg.type === "i128"
    ) {
      if (typeof value !== "string" && typeof value !== "number") {
        throw new Error(
          `${arg.name}: целочисленные значения указывайте строкой или числом.`,
        );
      }
      result[arg.name] = new anchor.BN(String(value));
    } else {
      result[arg.name] = value;
    }
  }
  return result;
}

function encodeRemainingAccounts(): anchor.web3.AccountMeta[] {
  let value: unknown;
  try {
    value = JSON.parse(textarea("remaining-accounts").value);
  } catch {
    throw new Error("Дополнительные CPI accounts: некорректный JSON.");
  }
  if (!Array.isArray(value)) {
    throw new Error("Дополнительные CPI accounts должны быть JSON-массивом.");
  }
  return value.map((item) => {
    if (!isRecord(item) || typeof item.pubkey !== "string") {
      throw new Error("Каждый remaining account должен содержать pubkey.");
    }
    return {
      pubkey: publicKey(item.pubkey, "Remaining account"),
      isWritable: item.isWritable === true,
      isSigner: item.isSigner === true,
    };
  });
}

async function executeAdvancedInstruction(): Promise<void> {
  const current = currentProgram();
  if (!vaultAdmin?.equals(currentWallet())) {
    throw new Error("Только администратор vault может выполнять стратегию.");
  }
  const name = document.getElementById("strategy-instruction");
  if (!(name instanceof HTMLSelectElement)) {
    throw new Error("Не выбрана инструкция.");
  }
  const instruction = current.idl.instructions.find(
    (candidate) => candidate.name === name.value,
  );
  if (!instruction) {
    throw new Error(`Инструкция ${name.value} отсутствует в IDL.`);
  }
  const args = encodeArguments(
    instruction,
    parseJsonObject("instruction-args", "Аргументы"),
  );
  const providedAccounts = parseJsonObject("instruction-accounts", "Аккаунты");
  const keys = instruction.accounts.map((account) => {
    if ("accounts" in account) {
      throw new Error(
        "Вложенные account-группы в этой форме не поддерживаются.",
      );
    }
    const address = providedAccounts[account.name];
    if (typeof address !== "string") {
      throw new Error(`В JSON не задан аккаунт ${account.name}.`);
    }
    return {
      pubkey: publicKey(address, account.name),
      isWritable: "writable" in account && account.writable === true,
      isSigner: "signer" in account && account.signer === true,
    };
  });
  const data = current.coder.instruction.encode(instruction.name, args);
  const ix = new TransactionInstruction({
    programId,
    keys: [...keys, ...encodeRemainingAccounts()],
    data,
  });
  await recordTransaction(
    instruction.name,
    currentProvider().sendAndConfirm(new Transaction().add(ix), []),
  );
}

function fillKnownAccounts(): void {
  const current = currentProgram();
  const owner = currentWallet();
  const addresses = getAddresses();
  const quoteMintText = input("quote-mint").value.trim();
  const poolText = input("pool-state").value.trim();
  const quoteMint = quoteMintText
    ? publicKey(quoteMintText, "Quote mint")
    : undefined;
  const accountMap: Record<string, PublicKey> = {
    admin: owner,
    user: owner,
    vault: addresses.vault,
    strategyAuthority: addresses.strategyAuthority,
    baseMint: addresses.baseMint,
    shareMint: addresses.shareMint,
    vaultTokenAccount: addresses.vaultTokenAccount,
    baseTokenAccount: addresses.vaultTokenAccount,
    tokenProgram: TOKEN_PROGRAM_ID,
    tokenProgram2022: TOKEN_2022_PROGRAM_ID,
    associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    systemProgram: SystemProgram.programId,
    clmmProgram: RAYDIUM_CLMM_DEVNET,
    rent: SYSVAR_RENT_PUBKEY,
  };
  if (quoteMint) {
    accountMap.quoteMint = quoteMint;
    accountMap.quoteTokenAccount = getAssociatedTokenAddressSync(
      quoteMint,
      addresses.strategyAuthority,
      true,
    );
  }
  if (poolText) {
    accountMap.poolState = publicKey(poolText, "Raydium pool");
  }
  const selected = document.getElementById("strategy-instruction");
  if (!(selected instanceof HTMLSelectElement)) {
    throw new Error("Не выбрана инструкция.");
  }
  const instruction = current.idl.instructions.find(
    (candidate) => candidate.name === selected.value,
  );
  if (!instruction) {
    throw new Error("Инструкция отсутствует в IDL.");
  }
  const template: Record<string, string> = {};
  for (const account of instruction.accounts) {
    if (!("accounts" in account) && account.name in accountMap) {
      template[account.name] = accountMap[account.name].toBase58();
    }
  }
  textarea("instruction-accounts").value = JSON.stringify(template, null, 2);
}

function configureAdvancedDefaults(): void {
  const selected = document.getElementById("strategy-instruction");
  if (!(selected instanceof HTMLSelectElement)) {
    return;
  }
  const args: Record<string, unknown> = {
    swap: { inputIsBase: true, amount: "1000000", minOutput: "990000" },
    openClmmPosition: {
      tickLowerIndex: -100,
      tickUpperIndex: 100,
      tickArrayLowerStartIndex: -600,
      tickArrayUpperStartIndex: 0,
      liquidity: "1000000",
      amount0Max: "1000000",
      amount1Max: "1000000",
    },
    decreaseLiquidity: {
      tickArrayLowerStartIndex: -600,
      tickArrayUpperStartIndex: 0,
      liquidity: "1000000",
      amount0Min: "0",
      amount1Min: "0",
    },
    collectPositionFees: {
      tickArrayLowerStartIndex: -600,
      tickArrayUpperStartIndex: 0,
    },
    closeClmmPosition: {},
  };
  textarea("instruction-args").value = JSON.stringify(
    args[selected.value] ?? {},
    null,
    2,
  );
}

async function disconnectWallet(): Promise<void> {
  await injectedWallet?.disconnect();
  injectedWallet = undefined;
  provider = undefined;
  program = undefined;
  vaultAdmin = undefined;
  setText("wallet-label", "Кошелёк не подключён");
  button("connect-wallet").hidden = false;
  button("disconnect-wallet").hidden = true;
  button("refresh-vault").disabled = true;
  button("initialize-vault").disabled = true;
  button("initialize-vault").hidden = false;
  button("open-position").disabled = true;
  button("open-position").hidden = true;
  button("deposit").disabled = true;
  button("withdraw").disabled = true;
  button("configure-strategy").disabled = true;
  button("begin-strategy").disabled = true;
  button("end-strategy").disabled = true;
  button("execute-instruction").disabled = true;
  button("fill-accounts").disabled = true;
  setText("address-list", "Подключите кошелёк и укажите mint токена.");
  setText("vault-balance", "—");
  setText("share-supply", "—");
  setText("user-balances", "—");
  setText("strategy-status", "—");
  setText("strategy-pool", "pool не настроен");
  const adminPanel = document.getElementById("admin-panel");
  if (adminPanel) {
    adminPanel.hidden = true;
  }
  showActivity("Кошелёк отключён.");
}

button("connect-wallet").addEventListener("click", () => {
  void runAction(connectWallet);
});
button("disconnect-wallet").addEventListener("click", () => {
  void runAction(disconnectWallet);
});
button("refresh-vault").addEventListener("click", () => {
  void runAction(refreshVault);
});
button("initialize-vault").addEventListener("click", () => {
  void runAction(initializeVault);
});
button("open-position").addEventListener("click", () => {
  void runAction(createUserPosition);
});
button("deposit").addEventListener("click", () => {
  void runAction(deposit);
});
button("withdraw").addEventListener("click", () => {
  void runAction(withdraw);
});
button("configure-strategy").addEventListener("click", () => {
  void runAction(configureStrategy);
});
button("begin-strategy").addEventListener("click", () => {
  void runAction(beginStrategy);
});
button("end-strategy").addEventListener("click", () => {
  void runAction(endStrategy);
});
button("execute-instruction").addEventListener("click", () => {
  void runAction(executeAdvancedInstruction);
});
button("fill-accounts").addEventListener("click", () => {
  void runAction(async () => {
    fillKnownAccounts();
    showActivity(
      "Подставлены адреса, которые можно вывести из текущих настроек.",
    );
  });
});
document
  .getElementById("strategy-instruction")
  ?.addEventListener("change", configureAdvancedDefaults);

configureAdvancedDefaults();
