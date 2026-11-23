# Liquidity Vault

Учебный проект Solana по методичке недель 8–15. Это **один Anchor workspace**: базовый vault расширяется следующими этапами в этой же программе и репозитории.

## Текущая реализация

- **Vault и доли:** vault PDA контролирует share mint; пользователь явно создает PDA-профиль инструкцией `open_position`.
- **Депозит/вывод:** пропорциональный выпуск и сжигание shares, переводы SPL-токенов через CPI, проверяемая арифметика.
- **Стратегия Raydium CLMM:** администратор может настроить пул с парой base/quote, начать стратегию, обменивать токены, открыть и уменьшить позицию, собрать комиссии, закрыть позицию и завершить стратегию.
- **Безопасность учета:** пока стратегия активна, депозиты и вывод заблокированы. Завершить стратегию можно только после закрытия позиции и обмена всего quote-баланса обратно в base.
- **PDA стратегии:** отдельный system-owned PDA владеет token account и оплачивает CPI Raydium; приватного ключа у него нет.

Rust unit-тесты проходят; TypeScript проверяется командой `npm run typecheck`. Production-сборка веб-интерфейса проходит, подключение Phantom и тестовые mint проверены. On-chain действия UI ещё не проверены, поскольку программа пока не развёрнута в Devnet. Devnet CLMM pool, реальные Raydium-транзакции и интеграционный тест с локальным валидатором не подтверждены. Не добавляйте приватные ключи или seed-фразы в репозиторий.

## Статус Devnet deploy

Программа успешно собрана с feature `devnet`; program ID и соответствующий keypair уже настроены. Повторно создавать keypair не нужно. Сам deploy пока не выполнен: на момент проверки на локальном signer было около **1,9 SOL**, тогда как аренда собранной программы стоит примерно **2,53 SOL** плюс комиссия. Сначала пополните signer; для deploy нужно больше 2,53 SOL с запасом на комиссию. Создание CLMM pool и добавление ликвидности потребуют дополнительных средств.

После пополнения проверьте, что CLI настроен на Devnet, и выполните deploy из корня проекта:

```powershell
solana config set --url devnet
solana program deploy target/deploy/liquidity_vault.so --program-id target/deploy/liquidity_vault-keypair.json --url devnet
```

До успешного deploy не используйте кнопки приложения, которые отправляют транзакции в vault. После deploy проверьте программу и заполните фактический ID и Explorer-ссылку в [журнале Devnet](docs/devnet.md). CLMM pool, позиция с ликвидностью и Raydium CPI-транзакции пока не созданы и не проверены.

## Аккаунты

```text
Vault PDA       [b"vault", base_mint]
Share Mint PDA  [b"share", vault]
Position PDA    [b"position", vault, user]
Strategy PDA    [b"strategy-authority", vault]
Vault ATA       associated token account(base_mint, strategy PDA)
Quote ATA       associated token account(quote_mint, strategy PDA)
User share ATA  associated token account(share_mint, user)
```

Администратор записан в аккаунте vault. Share mint authority — vault PDA; владелец token accounts и авторизованный плательщик Raydium CPI — system-owned strategy PDA. Программа подписывает от имени обоих PDA с их seeds. Владелец и количество shares определяются SPL token account, а не дублируемым счетчиком в PDA-профиле, поэтому переводы shares не ломают вывод.

## Структура репозитория

```text
programs/liquidity_vault/   Anchor-программа
tests/                      интеграционные тесты
app/                        Vite UI для Phantom и Devnet
idl/                        Anchor IDL и TypeScript тип
scripts/devnet/              test mint и CLMM pool setup
docs/                       план дальнейших этапов и результаты devnet
```

## Подготовка и тестирование

Нужны Rust, Solana CLI, Anchor 0.30.1 и Node.js. Установите JS-зависимости:

```powershell
npm install
```

Проверка Rust-кода и математики share:

```powershell
cargo test --workspace
cargo check --workspace
```

Только для нового workspace или если program keypair отсутствует выполните команды создания ниже. Для текущей Devnet-конфигурации keypair уже создан; **не запускайте `solana-keygen new` повторно**, иначе изменится program ID. Не добавляйте keypair в Git.

```powershell
New-Item -ItemType Directory -Force target/deploy
solana-keygen new --no-bip39-passphrase --outfile target/deploy/liquidity_vault-keypair.json
npm run sync:program-id
```

`npm run sync:program-id` синхронизирует Anchor program ID, обновляет адрес в IDL и пересоздает TypeScript-тип. Скрипт читает локальный keypair только для получения публичного адреса и не выводит/не сохраняет секрет. Проверки TypeScript: `npm run typecheck` и `npm run lint`.

Запуск интеграционных тестов:

```powershell
npm test
```

## Веб-интерфейс

```powershell
npm install
npm run dev
```

Откройте показанный Vite URL, установите Phantom и выберите Devnet. Приложение использует Devnet RPC и текущий IDL. Введите адрес base mint, затем подключите кошелёк: интерфейс покажет PDA и балансы. Создатель может инициализировать vault; пользователь может создать профиль (депозит также создаёт его автоматически), внести токены и вывести shares. Операции запрашивают подтверждение Phantom, а подпись ведёт на Solana Explorer.

Административная панель видна только кошельку, записанному как admin. Простые формы настраивают pool и блокировку стратегии. Для Raydium swap и управления CLMM-позицией предусмотрена расширенная форма: аккаунты, не выводимые из IDL и PDA, требуется заполнить адресами соответствующего Devnet pool; remaining CPI accounts передаются отдельным JSON-массивом. Сверяйте адреса и slippage-параметры перед подписью. Тестовые активы не имеют реальной стоимости.

## Подготовка Devnet-токенов и CLMM pool

Подготовительные скрипты требуют Node.js и локальный файл Solana keypair, который уже пополнен Devnet SOL. Они читают его из `ANCHOR_WALLET`, не показывают и не копируют содержимое; не отправляйте этот файл и не добавляйте его в Git. В PowerShell:

```powershell
$env:ANCHOR_WALLET = "$env:USERPROFILE\.config\solana\id.json"
$env:TOKEN_RECIPIENT = "<публичный адрес Phantom>"
npm run devnet:create-mints

$env:BASE_MINT = "<адрес BASE из вывода>"
$env:QUOTE_MINT = "<адрес QUOTE из вывода>"
$env:INITIAL_PRICE = "1"
npm run devnet:create-clmm-pool
```

Скрипт создаёт два SPL mint с 6 decimals и отправляет по 1000 тестовых токенов на ATA адреса из `TOKEN_RECIPIENT` (либо на адрес локального signer, если переменная не задана). Передавайте только публичный адрес Phantom; приватный ключ Phantom не экспортируйте. Второй скрипт использует официальный [Raydium SDK V2 demo](https://github.com/raydium-io/raydium-sdk-V2-demo) и создаёт пустой CLMM pool; `INITIAL_PRICE` задаётся в quote-токенах за один base-токен. Скрипты выводят только публичные адреса и Explorer-ссылку на подтверждённую mint/pool транзакцию. Создание пула не добавляет ликвидность: отдельно внесите оба тестовых актива в диапазон, затем используйте адреса и tick range в admin-проверках vault.

Для SBF-сборки Devnet-конфигурации используйте `anchor build --no-idl -- --features devnet`. Anchor 0.30.1 в текущем toolchain не генерирует IDL автоматически из-за зависимости от semver-exempt API `proc_macro2::Span::source_file`; текущие JSON- и TypeScript-IDL находятся в `idl/`. После изменения интерфейса сверяйте IDL с Rust API, затем пересоздавайте TypeScript-тип командой `npm run idl:types`.

Для devnet настройте кошелек локально, убедитесь в наличии тестовых SOL и токенов, затем выполните:

```powershell
solana config set --url devnet
```

Перед deploy создайте собственный program keypair и обновите program ID:

```powershell
solana-keygen new --no-bip39-passphrase --outfile target/deploy/liquidity_vault-keypair.json
npm run sync:program-id
anchor build --no-idl -- --features devnet
solana program deploy target/deploy/liquidity_vault.so --program-id target/deploy/liquidity_vault-keypair.json --url devnet
```

Не добавляйте wallet-файл, seed-фразу или keypair в Git. После успешного deploy внесите фактический program ID, PDA и ссылки на транзакции в `docs/devnet.md`.

## Этапы

- [План продолжения проекта](docs/ROADMAP.md)
- [Ответы для защиты](docs/concepts.md)
- [Devnet-адреса и транзакции](docs/devnet.md)
- [Сценарий демонстрации](docs/demo.md)
