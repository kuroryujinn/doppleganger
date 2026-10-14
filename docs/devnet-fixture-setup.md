# One-Time Devnet Fixture Preparation

The completed R11 live acceptance retains a genuine Vault capture under
`fixtures/devnet-snapshots/vault-devnet/`; see the acceptance report.
These are separate setup steps for preparing another disposable V1 fixture.
Do not repeat initialization for the published authority/program pair.
R11 itself never deploys, funds, initializes, or upgrades a remote program.

Use a separate scratch copy and a disposable **Devnet-only** fixture. Do not
change the accepted local fixture IDs or deploy the broken V2 candidates.
The public test seed below intentionally has no confidentiality. It is suitable
only for disposable test state; never use it for a wallet with real assets.
Keep your deployment wallet and program keypair private and outside artifacts.

## 1. Prepare a separate copy and program identity

Install the repository's Rust/Solana/Anchor prerequisites and Node.js 20.18+
for the one-time client. From the repository root:

```bash
export DG_DEVNET_FIXTURE="$(mktemp -d "$HOME/dg-devnet-fixture.XXXXXX")"
cp -a programs crates examples fixtures scripts Cargo.toml Cargo.lock Anchor.toml "$DG_DEVNET_FIXTURE/"
cd "$DG_DEVNET_FIXTURE"
NO_DNA=1 solana-keygen new --silent --no-bip39-passphrase --outfile "$DG_DEVNET_FIXTURE/program-keypair.json"
NO_DNA=1 solana-keygen new --silent --no-bip39-passphrase --outfile "$DG_DEVNET_FIXTURE/deployment-wallet.json"
solana-keygen pubkey "$DG_DEVNET_FIXTURE/program-keypair.json"
```

Record the printed program ID. In this scratch copy only, replace `declare_id!`
in **both** `programs/dg-vault-v1/src/lib.rs` and
`programs/dg-vault-v2/src/lib.rs` with that same ID. Update both Vault entries
in scratch `Anchor.toml` to match. Preserve all account and instruction logic.

## 2. Build local artifacts

```bash
mkdir -p target/idl
NO_DNA=1 cargo build-sbf --tools-version v1.57 --manifest-path programs/dg-vault-v1/Cargo.toml --sbf-out-dir target/deploy/v1
NO_DNA=1 cargo build-sbf --tools-version v1.57 --manifest-path programs/dg-vault-v2/Cargo.toml --sbf-out-dir target/deploy/v2
NO_DNA=1 cargo build-sbf --tools-version v1.57 --manifest-path programs/dg-vault-v2/Cargo.toml --features r7-broken-balance --sbf-out-dir target/deploy/v2-broken
NO_DNA=1 anchor idl build -p dg_vault_v1 -o target/idl/v1.json
NO_DNA=1 anchor idl build -p dg_vault_v2 -o target/idl/v2.json
```

On the current development machine, use `/home/kuro/.avm/bin/anchor-1.2.0`
instead of the `anchor` wrapper, as described in the repository README.
Verify both IDLs contain the new program ID. V2 stays local throughout R11.

## 3. Manually deploy V1 to Devnet

Use the freshly generated disposable deployment wallet explicitly. The next
commands mutate Devnet and are intentionally outside snapshot capture/replay:

```bash
NO_DNA=1 solana balance --url devnet --keypair "$DG_DEVNET_FIXTURE/deployment-wallet.json"
# Only request funding when the preceding balance is insufficient.
NO_DNA=1 solana airdrop 2 --url devnet --keypair "$DG_DEVNET_FIXTURE/deployment-wallet.json"
NO_DNA=1 solana program deploy --url devnet \
  --keypair "$DG_DEVNET_FIXTURE/deployment-wallet.json" \
  --upgrade-authority "$DG_DEVNET_FIXTURE/deployment-wallet.json" \
  --use-tpu-client \
  --program-id "$DG_DEVNET_FIXTURE/program-keypair.json" \
  target/deploy/v1/dg_vault_v1.so
solana program show --url devnet PROGRAM_ID
```

Replace `PROGRAM_ID` with the recorded public ID. Deployment may require more
Devnet SOL than one faucet request provides. The deployed ID must match the
V1 binary's `declare_id!`, the IDL, and both manifest version IDs.
Do not deploy V2 or perform a remote upgrade.
If funding fails, stop with `LIVE_DEVNET_SETUP_BLOCKED`; a fresh program ID is
not proof of a deployment. The public faucet at https://faucet.solana.com can
be used manually, but do not substitute mainnet funds or a personal wallet.
The completed acceptance used CLI TPU upload after RPC-only upload attempts
hit connection limits. Preserve the same buffer if resuming a partial upload;
record all confirmed loader transactions rather than counting deployment as a
single transaction. Do not use `--skip-preflight`.

## 4. Prepare an explicitly replayable authority

In a separate scratch client directory, install Kit for this manual setup:

```bash
mkdir -p client
cd client
npm init -y
npm install @solana/kit
node --input-type=module -e 'import {createKeyPairSignerFromPrivateKeyBytes} from "@solana/kit"; console.log((await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(73))).address)'
```

Record the printed `OWNER_ADDRESS`. It corresponds exactly to a local
`test_seed` containing 32 copies of `73`. Manually fund that public disposable
test address with Devnet SOL:

```bash
solana airdrop 1 OWNER_ADDRESS --url devnet
solana balance OWNER_ADDRESS --url devnet
```

Check the owner balance before funding. If the disposable deployment wallet
already has sufficient Devnet SOL, the completed acceptance used this alternative
to another faucet request (0.01 SOL covered initialize, deposit, and fees):

```bash
NO_DNA=1 solana transfer OWNER_ADDRESS 0.01 --url devnet \
  --keypair "$DG_DEVNET_FIXTURE/deployment-wallet.json" --allow-unfunded-recipient
```

This uses a new fixture identity, not the public identities in the accepted
local-only example manifests. No private wallet key is imported into R11.

## 5. Initialize and deposit manually

Create `prepare.mjs` in the scratch client directory with this one-time client.
It sends exactly two transactions to **Devnet**: Vault V1 initialize and a
1,000,000-lamport deposit. Review those actions before running it. This client
is preparation documentation, not part of the R11 adapter or its normal flow.

```javascript
import fs from "node:fs";
import {
  AccountRole, address, appendTransactionMessageInstruction,
  createKeyPairSignerFromPrivateKeyBytes, createSolanaRpc,
  createSolanaRpcSubscriptions, createTransactionMessage, getAddressEncoder,
  getBase64EncodedWireTransaction, getProgramDerivedAddress, getSignatureFromTransaction,
  pipe, sendAndConfirmTransactionFactory,
  setTransactionMessageFeePayerSigner, setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
} from "@solana/kit";

const programAddress = address(process.argv[2]);
const idl = JSON.parse(fs.readFileSync(process.argv[3], "utf8"));
if (idl.address !== programAddress) throw new Error("IDL/program ID mismatch");
const rpc = createSolanaRpc("https://api.devnet.solana.com");
if (await rpc.getGenesisHash().send() !== "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG") {
  throw new Error("Not Devnet");
}
const owner = await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(73));
const [vault] = await getProgramDerivedAddress({
  programAddress,
  seeds: [new TextEncoder().encode("vault"), getAddressEncoder().encode(owner.address)],
});
const send = sendAndConfirmTransactionFactory({
  rpc, rpcSubscriptions: createSolanaRpcSubscriptions("wss://api.devnet.solana.com"),
});
for (const name of ["initialize", "deposit"]) {
  const definition = idl.instructions.find(i => i.name === name);
  if (!definition) throw new Error(`Missing ${name} instruction`);
  const data = Buffer.alloc(definition.discriminator.length + (name === "deposit" ? 8 : 0));
  data.set(definition.discriminator);
  if (name === "deposit") data.writeBigUInt64LE(1000000n, definition.discriminator.length);
  const { value: blockhash } = await rpc.getLatestBlockhash({ commitment: "confirmed" }).send();
  const message = pipe(
    createTransactionMessage({ version: 0 }),
    m => setTransactionMessageFeePayerSigner(owner, m),
    m => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
    m => appendTransactionMessageInstruction({ programAddress, data, accounts: [
      { address: vault, role: AccountRole.WRITABLE },
      { address: owner.address, role: AccountRole.WRITABLE_SIGNER, signer: owner },
      { address: address("11111111111111111111111111111111"), role: AccountRole.READONLY },
    ] }, m),
  );
  const transaction = await signTransactionMessageWithSigners(message);
  const signature = getSignatureFromTransaction(transaction);
  const simulation = await rpc.simulateTransaction(getBase64EncodedWireTransaction(transaction), {
    encoding: "base64", sigVerify: true, commitment: "confirmed",
  }).send();
  if (simulation.value.err !== null) throw new Error(`${name} simulation failed`);
  await send(transaction, { commitment: "finalized", abortSignal: AbortSignal.timeout(120000) });
  console.log(JSON.stringify({ name, signature, commitment: "finalized" }));
}
console.log(JSON.stringify({ programId: programAddress, owner: owner.address, vault }));
```

Run it once, replacing the public program ID:

```bash
node prepare.mjs PROGRAM_ID ../target/idl/v1.json
```

Record the final public addresses. A second initialize attempts to create an
existing account, so do not repeat preparation for that authority/program pair.
Wait for finalization before capturing the accounts with the finalized adapter.

## 6. Configure capture and local replay

Return to the scratch project root. Copy `examples/vault-runtime` to
`examples/vault-runtime-devnet`, then edit its `doppelganger.toml`:

1. Set both `versions.v1.program_id` and `versions.v2.program_id` to `PROGRAM_ID`.
2. Set `addresses.owner.test_seed` to an array of 32 copies of `73`.
3. Set `addresses.stranger.test_seed` to an array of 32 copies of `74`.
4. Keep local artifact paths, PDA bindings, migration, preservation, and lifecycle definitions.
5. Append the configuration below. The `$vault` derivation should match the recorded vault address.

```toml
[snapshot]
source = "devnet"

[[snapshot.accounts]]
name = "vault"
address = "$vault"

[[snapshot.accounts]]
name = "owner"
address = "$owner"
owner = "$system_program"
```

The fixture initialization/setup entries are never executed by snapshot replay.
The stored owner balance is imported exactly; it is not funded by R11.

```bash
cargo run -p dg-cli -- snapshot capture examples/vault-runtime-devnet --output .doppelganger/snapshots/vault-devnet
cargo run -p dg-cli -- snapshot inspect .doppelganger/snapshots/vault-devnet
cargo run -p dg-cli -- snapshot replay .doppelganger/snapshots/vault-devnet \
  --project examples/vault-runtime-devnet --lifecycle --output report/devnet-replay.json
```

After capture, disconnect network access and rerun replay to establish the
offline acceptance evidence. Preserve `snapshot.json` and the local binaries
and IDLs; do not include the deployment wallet or program keypair in artifacts.

References: [Solana deployment](https://solana.com/docs/programs/deploying),
[Kit deterministic test signer](https://www.solanakit.com/api/functions/createKeyPairSignerFromPrivateKeyBytes),
[Kit PDA derivation](https://www.solanakit.com/api/functions/getProgramDerivedAddress),
[Kit transaction confirmation](https://www.solanakit.com/api/functions/sendAndConfirmTransactionFactory).
