# Devnet deployment log

Этот файл — журнал доказательств, а не шаблон для вымышленных результатов. Заполняйте его только после фактически подтверждённых транзакций.

## Развёртывание и аккаунты

```text
Program ID:
Program deployment transaction:
Vault PDA:
Strategy authority PDA:
Base token mint:
Quote token mint:
Share mint:
Vault token account:
Quote token account:
Raydium CLMM pool:
Pool current price:
Chosen tick range and rationale:
Position NFT mint:
```

## Созданные test mint

Оба mint созданы в Devnet; по 1000 токенов отправлено на Phantom `8wJ22ffvZU3Hq43SX15Ts7CfMdd1PAaQ42bDBJCiaJB8`.

| Токен | Mint                                                                                                                                            | Подтверждённая транзакция                                                                                                                          |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| BASE  | [AmG8oxvrEU7C3aKdpKRBqknpXL19ebB6JQiFhwAnn8XG](https://explorer.solana.com/address/AmG8oxvrEU7C3aKdpKRBqknpXL19ebB6JQiFhwAnn8XG?cluster=devnet) | [Explorer](https://explorer.solana.com/tx/5Y93GdafM1bQ9PQNnzXiLNzBixHLP9P5JF9KTpN7Vt1tmC6hy54qgJTw8xLHC6TbdKqLvbfwni1z6DGsryiaAdFy?cluster=devnet) |
| QUOTE | [5TKGD6mnYApiG8bQUw1xUA8hmTKEhPLkvBhvKyDF3dBR](https://explorer.solana.com/address/5TKGD6mnYApiG8bQUw1xUA8hmTKEhPLkvBhvKyDF3dBR?cluster=devnet) | [Explorer](https://explorer.solana.com/tx/2ynUidWqgKTuWoNG2pwx3eFdB7ummgWPcUHVt8nMdJK6y2UqDtuNvAE6HazbCWfSU18c98M8giowP9SrhsT8zX8c?cluster=devnet) |

## Транзакции и наблюдения

| Проверка                                 | Подтверждённая Explorer-ссылка | Результат / фактические значения |
| ---------------------------------------- | ------------------------------ | -------------------------------- |
| Vault initialize                         | Не выполнено                   |                                  |
| Deposit и выпуск shares                  | Не выполнено                   |                                  |
| Withdraw и burn shares                   | Не выполнено                   |                                  |
| Swap с допустимым slippage               | Не выполнено                   |                                  |
| Swap отклонён из-за min output           | Не выполнено                   |                                  |
| Цена до/после swap                       | Не выполнено                   |                                  |
| Открытие CLMM позиции                    | Не выполнено                   |                                  |
| Сбор комиссий                            | Не выполнено                   |                                  |
| Закрытие позиции                         | Не выполнено                   |                                  |
| Возврат quote в base и снятие блокировки | Не выполнено                   |                                  |

Все ссылки должны указывать на Solana Explorer с `cluster=devnet`. Адреса тестовых mint, pool, ticks, цены, суммы и результаты указывайте только после проверки в сети. Не вставляйте секретный ключ или seed-фразу.

## Запуск UI

Из корня проекта:

```powershell
npm install
npm run dev
```

Приложение подключается к Phantom и отправляет транзакции на Solana Devnet. Для инициализации vault нужен SPL base mint; для админской стратегии нужны адреса quote mint и Raydium CLMM pool. Расширенные CPI-инструкции требуют account map из Devnet pool, а также оставшиеся tick-array accounts. Транзакцию всегда проверяйте в Phantom до подписания.

## Подготовка тестовых mint и CLMM pool

Из PowerShell задайте путь к своему локальному keypair и запустите скрипты. Не отправляйте файл кошелька и не коммитьте его:

```powershell
$env:ANCHOR_WALLET = "$env:USERPROFILE\.config\solana\id.json"
$env:TOKEN_RECIPIENT = "<публичный адрес Phantom>"
npm run devnet:create-mints

$env:BASE_MINT = "<адрес BASE из вывода>"
$env:QUOTE_MINT = "<адрес QUOTE из вывода>"
$env:INITIAL_PRICE = "1"
npm run devnet:create-clmm-pool
```

Mint-скрипт создаёт два токена с 6 decimals и отправляет по 1000 единиц на адрес из `TOKEN_RECIPIENT` (если он не задан — локальному signer). В `TOKEN_RECIPIENT` указывайте только публичный адрес Phantom; seed-фраза или приватный ключ не нужны. Pool-скрипт основан на официальном [Raydium SDK V2 demo](https://github.com/raydium-io/raydium-sdk-V2-demo), выбирает первый CLMM fee config для Devnet и выводит публичные адреса pool, vaults, observation state и Explorer-ссылку. Он создаёт пустой пул; для позиции нужно отдельно добавить ликвидность обоих токенов и выбрать допустимые tick boundaries. `INITIAL_PRICE` — quote за один base.

## Текущие ограничения проверки

- `cargo test --workspace` проходит (4 теста). `npm test` остановился до integration test: Anchor не нашёл `agave-install` и не смог установить Agave 1.18.26 в этой Windows-среде.
- Программа ещё не развёрнута в Devnet; вызовы vault и Raydium CPI не выполнялись.
- Test mint созданы и проверены; CLMM pool, ручная LP-позиция, swap/slippage и сбор комиссий ещё не подтверждены.
- UI прошёл production build и проверку типов; Phantom подключён, но действия приложения с программой пока не проверялись.
- Для полного acceptance нужно заполнить журнал выше реальными Explorer-ссылками.
