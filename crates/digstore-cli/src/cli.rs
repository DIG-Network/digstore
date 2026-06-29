//! `clap` command-line surface for the `digstore` binary.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "digstore", version, about, long_about = None)]
pub struct Cli {
    /// Override the .dig metadata directory (default: the workspace's .dig).
    #[arg(long, global = true)]
    pub dig_dir: Option<PathBuf>,
    /// Emit machine-readable JSON instead of human-formatted output.
    #[arg(long, global = true)]
    pub json: bool,
    /// Enable verbose (debug-level) logging.
    #[arg(short, long, global = true)]
    pub verbose: bool,
    /// Color output: auto (default), always, or never.
    #[arg(long, global = true, default_value = "auto")]
    pub color: crate::ui::ColorChoice,
    /// Suppress progress and hints.
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// Operate on a specific store by name (overrides the active store).
    #[arg(long = "store", alias = "project", global = true)]
    pub store_name: Option<String>,
    /// Operating directory for add/urn/status (overrides the store's content root).
    #[arg(short = 'C', long = "cwd", global = true)]
    pub cwd: Option<PathBuf>,
    /// Never prompt; fail fast on missing required input. For automated / CI runs (also
    /// auto-enabled when stdin is not a terminal). Pair with --yes to auto-approve confirmations.
    #[arg(long, global = true)]
    pub non_interactive: bool,
    /// Assume "yes" to confirmation prompts. Required to proceed past a destructive/costly
    /// confirmation in non-interactive mode.
    #[arg(short = 'y', long, global = true)]
    pub yes: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start a new store from a template — free, no wallet, no spend.
    New(NewArgs),
    /// Preview your store locally — builds on save, serves the real chia://
    /// read path with live reload. Free, no chain, no spend.
    Dev(DevArgs),
    /// Check you're ready to publish (seed, funds, login, remote, content).
    Doctor(DoctorArgs),
    /// Create your store on Chia so you can publish it (mints on mainnet).
    Init(InitArgs),
    /// Stage files, directories, or glob patterns for your next publish.
    Add(AddArgs),
    /// Publish your staged files as a new version (a new on-chain capsule).
    Commit(CommitArgs),
    /// Build a hostable module + root from a directory, with NO chain/wallet
    /// (headless). The caller anchors the printed root on-chain separately.
    Compile(CompileArgs),
    /// Show the active store, its content folder, and unpublished changes.
    Status(StatusArgs),
    /// Show your store's publish history (each published capsule).
    Log(LogArgs),
    /// Show what changed between two published versions.
    Diff(DiffArgs),
    /// Save a published capsule's files into a local folder.
    Checkout(CheckoutArgs),
    /// Read a published file by its share link (URN) or retrieval key.
    Cat(CatArgs),
    /// Manage remote endpoints for this store (add, list, remove).
    Remote(RemoteArgs),
    /// Clone a store from a remote into the current directory.
    Clone(CloneArgs),
    /// Upload your store's content and signed head to a remote.
    Push(PushArgs),
    /// Pull the latest content and signed head from a remote.
    Pull(PullArgs),
    /// Revoke a published root (or the whole store) with a signed tombstone.
    Revoke(RevokeArgs),
    /// Run a dig:// remote node serving the active store (clone/pull/push, §21).
    Serve(ServeArgs),
    /// List the stores in this workspace.
    #[command(alias = "projects")]
    Stores(StoresArgs),
    /// Switch the active store by name.
    Use(UseArgs),
    /// Show or set the active store's content root directory.
    Dir(DirArgs),
    /// Clear the staging area (discard all staged entries).
    Unstage(UnstageArgs),
    /// List the files currently staged for the next commit.
    Staged(StagedArgs),
    /// Print the URN(s) for staged or committed resources.
    Urn(UrnArgs),
    /// List the retrieval key (and URN) for every committed resource.
    Keys(KeysArgs),
    /// Update DigStore to the latest release.
    Update(UpdateArgs),
    /// Manage the encrypted wallet seed in ~/.dig.
    Seed(SeedArgs),
    /// Lock the seed (clear the cached-unlock session).
    Lock(LockArgs),
    /// Resume or inspect the store's on-chain anchor.
    Anchor(AnchorArgs),
    /// Show wallet XCH + DIG balance.
    Balance(BalanceArgs),
    /// Log in to your DIGHUb account via device pairing.
    Login(LoginArgs),
    /// Show the current DIGHUb login (handle / token presence).
    Whoami(WhoamiArgs),
    /// Log out of DIGHUb (clear the stored session).
    Logout(LogoutArgs),
    /// Deploy a built site/dapp to an EXISTING store from CI (a new capsule),
    /// reading `dig.toml` — git-push-to-deploy. Never mints (no init).
    Deploy(DeployArgs),
    /// Manage the per-store publisher deploy key (export it once for CI; import it).
    DeployKey(DeployKeyArgs),
    /// Get set up to publish: seed (import/generate), fund check, optional login.
    #[command(visible_alias = "auth")]
    Setup(SetupArgs),
    /// Connect this folder to an existing store (writes dig.toml + remote).
    Link(LinkArgs),
    /// Print a shell completion script (bash, zsh, fish, powershell, elvish).
    Completion(CompletionArgs),
    /// Mint, transfer, and list NFTs (media stored permanently in DIG capsules).
    Nft(NftArgs),
    /// Create and bulk-mint NFT collections from a traits manifest.
    Collection(CollectionArgs),
    /// Create a creator-identity DID (decentralized identifier).
    Did(DidArgs),
    /// Make, take, and inspect Chia offers (XCH/CAT trades).
    Offer(OfferArgs),
}

// ===========================================================================
// Wave-B asset CLI (#35): nft / collection / did / offer.
//
// These surface the existing, Simulator-tested `digstore-chain` builders to the
// terminal. Each on-chain action builds the spend via a chain builder, signs
// with the wallet seed (the same unlock path as `commit`/`balance`), and pushes
// via coinset. `--json` is non-interactive/CI-safe. `--dry-run` (where present)
// BUILDS the spend but never signs/pushes — so the offline suite can exercise
// the build path without spending. The capsule-media NFT mint (#33) additionally
// writes the art + CHIP-0007 metadata into a real DIG capsule first.
// ===========================================================================

#[derive(Debug, Args)]
#[command(
    after_help = "Each on-chain action spends an XCH network fee (and mints cost 1 mojo for the \
singleton). Signed with your wallet seed; pushed via coinset.\n\nThe `mint` path stores the art + \
CHIP-0007 metadata PERMANENTLY in a DIG capsule (not a centralized URL) and pins the on-chain hashes \
to the real bytes — the \"truly permanent NFT\" path.\n\nEXAMPLES:\n  digstore nft mint --art \
./art.png --name \"DIG Punk #1\" --royalty 300\n  digstore nft mint --art ./art.png --name X \
--dry-run --json\n  digstore nft transfer --nft <launcher-id> --to <xch-address>\n  digstore nft list \
--json"
)]
pub struct NftArgs {
    #[command(subcommand)]
    pub action: NftAction,
}

#[derive(Debug, Subcommand)]
pub enum NftAction {
    /// Mint one NFT whose media lives permanently in a DIG capsule (#33).
    Mint(NftMintArgs),
    /// Bulk-mint many NFTs from one funding coin in a single bundle.
    Bulk(NftBulkArgs),
    /// Transfer an owned NFT to a new owner address.
    Transfer(NftTransferArgs),
    /// List the NFTs the wallet currently owns.
    List(NftListArgs),
}

#[derive(Debug, Args)]
pub struct NftMintArgs {
    /// Path to the media file (art) to store in the capsule and mint as the NFT's data.
    #[arg(long)]
    pub art: PathBuf,
    /// The NFT name (written into the CHIP-0007 metadata). Required.
    #[arg(long)]
    pub name: String,
    /// Optional NFT description (CHIP-0007 metadata).
    #[arg(long)]
    pub description: Option<String>,
    /// Royalty in basis points (300 = 3%); default 0.
    #[arg(long, default_value_t = 0)]
    pub royalty: u16,
    /// Optional creator DID launcher id (64-hex) to attribute the mint to.
    #[arg(long)]
    pub did: Option<String>,
    /// An https gateway base to use as the data/metadata fallback URI (e.g.
    /// `https://rpc.dig.net`). The on-chain `dig://` URN (a wire value, not the
    /// address you open) is always the primary URI.
    #[arg(long = "gateway")]
    pub gateway: Option<String>,
    /// Build the capsule + mint spend and print the plan WITHOUT signing or pushing
    /// (no spend). Use to preview the on-chain `dig://` URN, computed hashes, and cost.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct NftBulkArgs {
    /// Path to a JSON manifest: an array of items `[{name, description?, attributes?, media:{data_uris,
    /// data_hash, ...}}]` (already-parsed traits manifest; the capsule packing is the toolkit's job).
    #[arg(long)]
    pub manifest: PathBuf,
    /// Optional creator DID launcher id (64-hex) to attribute every mint to.
    #[arg(long)]
    pub did: Option<String>,
    /// Build the bulk-mint spend and print the plan WITHOUT signing or pushing.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct NftTransferArgs {
    /// The NFT to transfer, by its launcher id (64-hex) or `nft1…` id.
    #[arg(long)]
    pub nft: String,
    /// The recipient mainnet address (`xch1…`).
    #[arg(long)]
    pub to: String,
    /// Build the transfer spend and print the plan WITHOUT signing or pushing.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct NftListArgs {}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore collection create --name \"DIG Punks\" --id dig-punks \
--royalty 300\n  digstore collection mint --collection ./collection.json --manifest ./items.json \
--did <did> --dry-run"
)]
pub struct CollectionArgs {
    #[command(subcommand)]
    pub action: CollectionAction,
}

#[derive(Debug, Subcommand)]
pub enum CollectionAction {
    /// Define a collection (shared id/name/royalty) and write its definition JSON.
    Create(CollectionCreateArgs),
    /// Bulk-mint every item in a traits manifest into a collection, attributed to a DID.
    Mint(CollectionMintArgs),
    /// Show one collection's items, owners, and royalty — read from coinset (no third-party API).
    Show(CollectionShowArgs),
    /// List the collections this wallet holds items for (grouped by creator DID).
    List(CollectionListArgs),
}

#[derive(Debug, Args)]
pub struct CollectionShowArgs {
    /// The collection's creator DID launcher id (64-hex) — the on-chain identity its items
    /// are attributed to.
    #[arg(long)]
    pub did: String,
}

#[derive(Debug, Args)]
pub struct CollectionListArgs {}

#[derive(Debug, Args)]
pub struct CollectionCreateArgs {
    /// Human-readable collection name. Required.
    #[arg(long)]
    pub name: String,
    /// Stable collection id (defaults to a slug of the name).
    #[arg(long)]
    pub id: Option<String>,
    /// Shared royalty in basis points for every item (300 = 3%); default 0.
    #[arg(long, default_value_t = 0)]
    pub royalty: u16,
    /// Royalty recipient mainnet address (`xch1…`); defaults to the wallet's own address.
    #[arg(long = "royalty-address")]
    pub royalty_address: Option<String>,
    /// Write the collection definition JSON to this file instead of stdout.
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    // --- #40 DROP MECHANICS (SCAFFOLDED): these set the drop data model in the
    // collection definition. The model is committed; ENFORCEMENT in the mint path is
    // TODO (see digstore_chain::collection::Drop). ---
    /// DELAYED REVEAL: Unix epoch seconds before which items mint with placeholder
    /// metadata (real art swapped in at/after this time). Scaffolded — see Drop docs.
    #[arg(long = "reveal-at")]
    pub reveal_at: Option<u64>,
    /// ALLOWLIST: an address/DID permitted to mint during allowlist phases. Repeatable.
    /// Scaffolded — membership is recorded, not yet enforced at mint.
    #[arg(long = "allow")]
    pub allow: Vec<String>,
    /// PHASED: a mint phase as `name[:start_unix[:supply]]` (e.g. `allowlist:1800000000:100`).
    /// Repeatable, in order. Scaffolded — the schedule is recorded, not yet enforced.
    #[arg(long = "phase")]
    pub phase: Vec<String>,
    /// LAZY MINT: mint items on-demand at claim time instead of the full supply up-front.
    /// Scaffolded — records intent; the claim/lazy-mint flow is TODO.
    #[arg(long = "lazy-mint")]
    pub lazy_mint: bool,
}

#[derive(Debug, Args)]
pub struct CollectionMintArgs {
    /// Path to a collection definition JSON (from `collection create`).
    #[arg(long)]
    pub collection: PathBuf,
    /// Path to the items manifest JSON (array of parsed manifest items).
    #[arg(long)]
    pub manifest: PathBuf,
    /// The creator DID launcher id (64-hex) the collection is minted under. Required (a collection
    /// mint is DID-attributed).
    #[arg(long)]
    pub did: String,
    /// Build the bulk-mint spend and print the plan WITHOUT signing or pushing.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
#[command(
    after_help = "A DID is your on-chain creator identity. Attribute mints to it so collectors can \
verify who made an NFT.\n\nEXAMPLES:\n  digstore did create\n  digstore did create --dry-run --json"
)]
pub struct DidArgs {
    #[command(subcommand)]
    pub action: DidAction,
}

#[derive(Debug, Subcommand)]
pub enum DidAction {
    /// Create a new simple DID owned by the wallet.
    Create(DidCreateArgs),
}

#[derive(Debug, Args)]
pub struct DidCreateArgs {
    /// Build the create-DID spend and print the plan WITHOUT signing or pushing.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore offer make --offer 1000xch --request 100dig\n  digstore \
offer take --offer offer1...\n  digstore offer show --offer offer1..."
)]
pub struct OfferArgs {
    #[command(subcommand)]
    pub action: OfferAction,
}

#[derive(Debug, Subcommand)]
pub enum OfferAction {
    /// Make an offer: offer XCH/DIG and request XCH/DIG (prints an `offer1…` string).
    Make(OfferMakeArgs),
    /// Take an existing `offer1…` string with the wallet's funds.
    Take(OfferTakeArgs),
    /// Show what an `offer1…` string offers, requests, and costs to take (no spend).
    Show(OfferShowArgs),
}

#[derive(Debug, Args)]
pub struct OfferMakeArgs {
    /// Asset to OFFER, as `<amount><asset>` where asset is `xch` or `dig` (e.g. `1000xch`, `100dig`).
    /// Repeatable.
    #[arg(long = "offer")]
    pub offer: Vec<String>,
    /// Asset to REQUEST, as `<amount><asset>` (e.g. `100dig`). Repeatable.
    #[arg(long = "request")]
    pub request: Vec<String>,
    /// Network fee in mojos (default 0).
    #[arg(long, default_value_t = 0)]
    pub fee: u64,
}

#[derive(Debug, Args)]
pub struct OfferTakeArgs {
    /// The bech32 `offer1…` string to take.
    #[arg(long)]
    pub offer: String,
    /// Network fee in mojos (default 0).
    #[arg(long, default_value_t = 0)]
    pub fee: u64,
    /// Decode + price the offer and print the plan WITHOUT signing or pushing.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct OfferShowArgs {
    /// The bech32 `offer1…` string to inspect.
    #[arg(long)]
    pub offer: String,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Scaffolds a working store locally — NO wallet, NO chain, NO spend. Preview it \
with `digstore dev`, then publish it with `digstore deploy` when it's ready.\n\nTEMPLATES:\n  \
static-site        a plain HTML/CSS site (no build step)\n  vite-react         a Vite + React app \
(window.chia wired)\n  next-static        a statically-exported Next.js app\n  nft-drop           \
an NFT drop / mint page\n  dapp-window-chia   a minimal dapp using the window.chia wallet\n\n\
Prefer JS? `npm create dig-app` scaffolds the same `static-site` template (and more) from \
Node.\n\nEXAMPLES:\n  digstore new static-site\n  digstore new vite-react ./my-app\n  digstore new \
dapp-window-chia ./dapp --force"
)]
pub struct NewArgs {
    /// Which template to scaffold (static-site, vite-react, next-static, nft-drop, dapp-window-chia).
    pub template: String,
    /// Target directory to create the store in (default: the current directory).
    pub dir: Option<PathBuf>,
    /// Write into a non-empty directory (overwriting any same-named files).
    #[arg(long)]
    pub force: bool,
    /// List the available templates and exit.
    #[arg(long)]
    pub list: bool,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Builds your store on every save and serves it over the REAL chia:// read path \
locally (compile → verify → decrypt, exactly as a visitor's browser does), with live reload and an \
injected dev `window.chia` wallet shim. FREE — no wallet, no chain, no spend.\n\nReads `output-dir` \
and `build-command` from `dig.toml` (flags override). Open the printed http://127.0.0.1:<port> URL.\n\n\
EXAMPLES:\n  digstore dev\n  digstore dev --dir dist --port 5000\n  digstore dev --build \"npm run \
build\" --open"
)]
pub struct DevArgs {
    /// The content/output directory to serve. Overrides `dig.toml`'s `output-dir` (default `.`).
    #[arg(long = "dir", visible_alias = "output-dir")]
    pub dir: Option<String>,
    /// A build command to run before each (re)build. Overrides `dig.toml`'s `build-command`.
    #[arg(long = "build", visible_alias = "build-command")]
    pub build_command: Option<String>,
    /// Port to bind the local preview server to (default 4343).
    #[arg(long, default_value_t = 4343)]
    pub port: u16,
    /// Open the preview URL in your default browser once it's serving.
    #[arg(long)]
    pub open: bool,
    /// Seconds between filesystem polls for the watch loop (default 1).
    #[arg(long, default_value_t = 1)]
    pub poll: u64,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Runs pre-publish checks so a costly on-chain publish doesn't fail halfway: is \
your seed present + unlocked, do you have enough $DIG + XCH for a publish, are you logged in to \
DIGHUb, is the default remote reachable, and does your content directory exist. Prints each as \
pass/fail and exits non-zero if any hard check fails.\n\nEXAMPLES:\n  digstore doctor\n  digstore \
doctor --json"
)]
pub struct DoctorArgs {}

#[derive(Debug, Args)]
#[command(
    after_help = "Costs a uniform per-capsule price (paid in $DIG at the live rate) + an XCH fee, paid on-chain at mint.\n\nEXAMPLES:\n  digstore init\n  digstore init site --dir dist\n  digstore init --private"
)]
pub struct InitArgs {
    /// Store name (default: "default").
    pub name: Option<String>,
    /// Display name written to the on-chain store metadata (shown in DIGHUb). Optional —
    /// prompted at init if not given; when left unset, displays fall back to the store id.
    #[arg(long)]
    pub label: Option<String>,
    /// Store description written to the on-chain metadata (optional).
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long)]
    pub private: bool,
    /// Content root (the build-output directory this store captures).
    #[arg(long)]
    pub dir: Option<String>,
    /// $DIG to pay for this mint, as a DIG amount (e.g. `100` or `87.5`; max 3 dp).
    /// Pricing is dynamic + USD-pegged — the hub computes the live amount and you
    /// pass it here; the CLI is deterministic and never fetches a price. Falls back
    /// to a protocol default if unset. Precedence: this flag > `DIGSTORE_DIG_AMOUNT`
    /// > dig.toml `dig-amount`.
    #[arg(long = "dig-amount", value_name = "DIG", value_parser = parse_dig_amount)]
    pub dig_amount: Option<u64>,
    /// Seconds to wait for on-chain confirmation (default 300; 0 = a single
    /// check, do not block). On a timeout the store is kept resumable; run
    /// `digstore anchor` to resume.
    #[arg(long, default_value_t = 300)]
    pub wait_timeout: u64,
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore add file.txt\n  digstore add -A\n  digstore add . src/*.rs\n  digstore add logo.png --key assets/logo.png"
)]
pub struct AddArgs {
    /// Files, directories, or glob patterns to stage (relative to the store root).
    pub paths: Vec<PathBuf>,
    /// Stage every file under the store root (honoring .digignore/.gitignore).
    #[arg(short = 'A', long)]
    pub all: bool,
    /// Show what would be staged without staging anything.
    #[arg(long)]
    pub dry_run: bool,
    /// Resource key override (only valid with exactly one file path).
    #[arg(long)]
    pub key: Option<String>,
    /// Stage the /.well-known/dig/manifest.json discovery manifest.
    #[arg(long)]
    pub discovery: bool,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Publishes your staged files as a new version on Chia — a new capsule \
(`storeId:rootHash`). Costs a uniform per-capsule price (paid in $DIG at the live rate) + an XCH \
fee per publish.\n\nUse `--dry-run` to preview the \
resulting version (root) and the exact DIG/XCH cost WITHOUT spending or publishing anything.\n\n\
TWO KINDS OF DEPLOY KEY (don't mix them up):\n  --writer-key  the ON-CHAIN ROOT-ADVANCE authority \
— a revocable WRITER DELEGATE that advances the store's root WITHOUT the owner master seed (the \
hub Teams \"Deployer\" flow pre-authorizes it). This command advances the root, so it takes \
--writer-key. Env: DIGSTORE_WRITER_KEY.\n  --deploy-key  (a DIFFERENT key) the §21 HUB HEAD-PUSH \
key — lets DIGHUb ACCEPT the capsule; used by `digstore deploy`, NOT here. From `digstore \
deploy-key export`. Env: DIGSTORE_DEPLOY_KEY.\n\nEXAMPLES:\n  digstore commit -m \"first \
version\"\n  digstore commit --dry-run\n  digstore commit -m deploy --writer-key $DIGSTORE_WRITER_KEY"
)]
pub struct CommitArgs {
    #[arg(short, long)]
    pub message: Option<String>,
    /// Preview the resulting version (root) + exact DIG/XCH cost WITHOUT spending,
    /// anchoring, or finalizing anything. Nothing is published.
    #[arg(long)]
    pub dry_run: bool,
    /// Seconds to wait for on-chain confirmation (default 300; 0 = a single
    /// check, do not block). On a timeout the local generation is NOT finalized
    /// and a resumable pending anchor is left; re-run `digstore commit` to finish.
    #[arg(long, default_value_t = 300)]
    pub wait_timeout: u64,
    /// Ignore an in-flight pending update and submit a fresh one (spends DIG +
    /// an XCH fee). Use when a previous commit's update is stuck and will not
    /// confirm. Without it, a re-run reuses the pending update.
    #[arg(long)]
    pub resubmit: bool,
    /// $DIG to pay for this publish, as a DIG amount (e.g. `100` or `87.5`; max 3 dp).
    /// Pricing is dynamic + USD-pegged — the hub computes the live amount and you pass
    /// it here; the CLI is deterministic and never fetches a price. Falls back to a
    /// protocol default if unset.
    /// Precedence: this flag > `DIGSTORE_DIG_AMOUNT` > dig.toml `dig-amount`.
    #[arg(long = "dig-amount", value_name = "DIG", value_parser = parse_dig_amount)]
    pub dig_amount: Option<u64>,
    /// After the deployment confirms, push it to DIGHUb (the default remote) WITHOUT
    /// asking. For scripting/CI. Mutually exclusive with `--no-push`. Default: ask
    /// when interactive, do nothing when not.
    #[arg(long, conflicts_with = "no_push")]
    pub push: bool,
    /// Never ask to push and never push after the deployment confirms (keeps only the
    /// `digstore push origin` hint). Mutually exclusive with `--push`.
    #[arg(long)]
    pub no_push: bool,
    /// Advance the on-chain root signed by a WRITER DELEGATE key (a revocable CI deploy
    /// token, 64-hex seed) instead of the owner master seed. The store owner must have
    /// pre-authorized this writer (via the hub Teams "Deployer" / `updateStoreOwnership`);
    /// the writer can change ONLY the metadata root — never the owner, never melt. Prefer
    /// the `DIGSTORE_WRITER_KEY` env var in CI so it is not visible in the process table.
    /// The wallet seed still pays the per-capsule $DIG price + XCH fee.
    ///
    /// `--deploy-key` is a DEPRECATED hidden alias for this flag (it used to mean the
    /// writer key here, which clashed with `deploy`'s publisher `--deploy-key`); use
    /// `--writer-key`. See `digstore deploy --help` for the writer-vs-publisher contrast.
    #[arg(long = "writer-key", alias = "deploy-key", value_name = "WRITER_SEED")]
    pub writer_key: Option<String>,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Headless: NO wallet, NO chain, NO signing. Stages <in>, computes the\ngeneration root, and writes the compiled module to <out>. The caller anchors\nthe printed root on-chain (e.g. via a wallet) separately.\n\nEXAMPLES:\n  digstore compile --in ./content --out ./module.dig --store-id <64-hex> --json"
)]
pub struct CompileArgs {
    /// Directory of files to compile into the store's capsule (the content root).
    #[arg(long)]
    pub r#in: PathBuf,
    /// Path to write the compiled module to.
    #[arg(long)]
    pub out: PathBuf,
    /// The on-chain store id (launcher id, 64-hex) this deployment belongs to.
    #[arg(long = "store-id")]
    pub store_id: String,
    /// Compile as a private (salted) store. Provide --salt for a deterministic root.
    #[arg(long)]
    pub private: bool,
    /// 32-byte hex SecretSalt for a private store (makes the root deterministic).
    /// Implies --private.
    #[arg(long)]
    pub salt: Option<String>,
    /// Optional path to a JSON metadata manifest (the dighub `Manifest` shape: name, version,
    /// description, authors, license, homepage, repository, keywords, categories, icon,
    /// content_type, links, custom) to embed in the module's data section and serve ungated via
    /// `get_metadata` (Digstore §8.4). Omitted => an empty manifest is embedded.
    #[arg(long)]
    pub metadata: Option<PathBuf>,
    /// The 48-byte hex BLS public key of the host that will SERVE this module (Digstore §12.2
    /// attestation gate). When set, the compiled module's trusted host-key set is the given key
    /// instead of a freshly-generated local one, so a delegated serving node (e.g. the DIGHUb
    /// retrieval host) can attest and release real content — without it, that host fails the gate
    /// and `serve_blind` returns indistinguishable decoys for every resource. Re-keys ONLY the
    /// TrustedKeys section (chunks/merkle/root preserved byte-for-byte → the generation root is
    /// unchanged; the program hash changes because the embedded key changed).
    #[arg(long = "host-key")]
    pub host_key: Option<String>,
    /// Treat each input file as the resource's ALREADY-SEALED ciphertext (sealed client-side under
    /// its per-URN key), not plaintext. The compiler stores each as a single chunk and skips
    /// chunking + encryption — so the server assembles the `.dig` from ciphertext alone and never
    /// sees plaintext or any decryption key. The read path is unchanged (one chunk per resource).
    #[arg(long = "pre-encrypted")]
    pub pre_encrypted: bool,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore status")]
pub struct StatusArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore log\n  digstore log --limit 10")]
pub struct LogArgs {
    #[arg(short, long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore diff <rootA> <rootB>")]
pub struct DiffArgs {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore checkout <root> --out ./out")]
pub struct CheckoutArgs {
    pub root: String,
    #[arg(long, short)]
    pub out: PathBuf,
    #[arg(long)]
    pub salt: Option<String>,
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore cat urn:dig:chia:<storeID>:<root>/readme\n  digstore cat urn:dig:chia:<storeID>/logo.png --out logo.png\n  digstore cat <64-hex-retrieval-key> --out blob.enc"
)]
pub struct CatArgs {
    /// A `urn:dig:…` (streamed out DECRYPTED) or a 64-char hex retrieval key
    /// (streamed out as RAW ENCRYPTED bytes, resolved within the active store).
    pub urn: String,
    /// Write output to this file instead of stdout.
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    /// Decryption salt (32-byte hex) for a private store.
    #[arg(long)]
    pub salt: Option<String>,
    /// Verify the resource's merkle proof against the trusted root before output.
    #[arg(long)]
    pub verify_proof: bool,
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore remote add origin https://<username>@rpc.dig.net\n\nThe store id is taken from the local store on push/pull, so the origin omits it."
)]
pub struct RemoteArgs {
    #[command(subcommand)]
    pub action: RemoteAction,
}

#[derive(Debug, Subcommand)]
pub enum RemoteAction {
    /// Add a remote. In interactive mode, name/url are prompted when omitted.
    Add {
        name: Option<String>,
        url: Option<String>,
    },
    List,
    Remove {
        name: String,
    },
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore clone https://host/stores/<storeID>")]
pub struct CloneArgs {
    pub source: String,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore push origin")]
pub struct PushArgs {
    #[arg(default_value = "origin")]
    pub remote: String,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Pulls EITHER a whole store from a remote, OR a single resource by URN over the\nnetwork (fetch ciphertext + merkle proof by retrieval key, verify against the\ntrusted root, auto-decrypt with the URN-derived key, write the plaintext).\n\nEXAMPLES:\n  digstore pull origin\n  digstore pull urn:dig:chia:<storeID>/docs/readme.md\n  digstore pull urn:dig:chia:<storeID>/logo.png --out logo.png\n  digstore pull urn:dig:chia:<storeID>:<root>/index.html --out index.html"
)]
pub struct PullArgs {
    /// A configured remote name (default `origin`) for a whole-store sync, OR a `urn:dig:…/<path>`
    /// resource URN for a network content-read by retrieval key.
    #[arg(default_value = "origin")]
    pub remote: String,
    /// For a resource URN: write the decrypted plaintext to this path (default: a file named after
    /// the resource key's last path segment in the current directory).
    #[arg(long, short)]
    pub out: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore revoke --root <hex> --reason compromise\n  digstore revoke --all --reason takedown\n  digstore revoke --root <hex> --remote origin"
)]
pub struct RevokeArgs {
    /// Revoke a single generation root (hex). Mutually exclusive with `--all`.
    #[arg(long, conflicts_with = "all")]
    pub root: Option<String>,
    /// Revoke the whole store (Store-scoped tombstone). Mutually exclusive with `--root`.
    #[arg(long)]
    pub all: bool,
    /// Why the root/store is revoked: unspecified (default), compromise, superseded, takedown.
    #[arg(long, default_value = "unspecified")]
    pub reason: String,
    /// The configured remote to publish the tombstone to.
    #[arg(default_value = "origin")]
    pub remote: String,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Runs a dig:// remote NODE for the active store: serves clone/pull/push\nover the §21 protocol (the same one rpc.dig.net speaks), so anyone can host\nan origin. Every request must be authenticated by a signed message from the\ncaller's identity key (§21.9).\n\nEXAMPLES:\n  digstore serve --bind 0.0.0.0:8443\n  digstore serve --store site --bind 127.0.0.1:9000"
)]
pub struct ServeArgs {
    /// Address to bind the node to (host:port).
    #[arg(long, default_value = "127.0.0.1:8443")]
    pub bind: String,
    /// Serve anonymously (a fully-public read mirror): skip §21.9 request auth.
    /// Off by default — the node requires a signed request from every caller.
    #[arg(long)]
    pub anonymous: bool,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore stores")]
pub struct StoresArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore use site")]
pub struct UseArgs {
    pub name: String,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore dir\n  digstore dir dist")]
pub struct DirArgs {
    pub path: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore unstage")]
pub struct UnstageArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore staged")]
pub struct StagedArgs {}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore urn -A\n  digstore urn css/app.css\n  digstore urn file --root <hex>"
)]
pub struct UrnArgs {
    pub paths: Vec<PathBuf>,
    #[arg(short = 'A', long)]
    pub all: bool,
    #[arg(long)]
    pub root: Option<String>,
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore keys\n  digstore keys --root <hex>\n  digstore keys --json"
)]
pub struct KeysArgs {
    /// Deployment root to list (hex); defaults to the current root.
    #[arg(long)]
    pub root: Option<String>,
}

#[derive(Debug, Args)]
pub struct SeedArgs {
    #[command(subcommand)]
    pub action: SeedAction,
}

#[derive(Debug, Subcommand)]
pub enum SeedAction {
    /// Import an existing BIP-39 mnemonic.
    Import {
        /// Provide the mnemonic non-interactively (otherwise prompted).
        #[arg(long)]
        mnemonic: Option<String>,
    },
    /// Generate a new BIP-39 mnemonic.
    Generate {
        /// Word count (12/15/18/21/24).
        #[arg(long, default_value_t = 24, value_parser = parse_word_count)]
        words: usize,
    },
    /// Show whether a seed exists and is currently unlocked.
    Status,
}

/// clap value parser for `--dig-amount`: a human DIG decimal string (max 3 dp) →
/// base units. Rejects `0` (a capsule must pay the protocol fee) and malformed input.
fn parse_dig_amount(s: &str) -> Result<u64, String> {
    let units = digstore_chain::dig::parse_dig(s)?;
    if units == 0 {
        return Err("dig amount must be greater than 0".to_string());
    }
    Ok(units)
}

fn parse_word_count(s: &str) -> Result<usize, String> {
    let n: usize = s.parse().map_err(|_| format!("`{s}` is not a number"))?;
    match n {
        12 | 15 | 18 | 21 | 24 => Ok(n),
        _ => Err("word count must be one of 12, 15, 18, 21, 24".to_string()),
    }
}

#[derive(Debug, Args)]
pub struct LockArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore balance\n  digstore balance --json")]
pub struct BalanceArgs {}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore anchor\n  digstore anchor status\n  digstore anchor --wait-timeout 600"
)]
pub struct AnchorArgs {
    /// `status` to inspect read-only; omitted to resume a pending anchor.
    #[command(subcommand)]
    pub action: Option<AnchorAction>,
    /// Seconds to wait for on-chain confirmation when resuming (default 300;
    /// 0 = a single check, do not block).
    #[arg(long, default_value_t = 300)]
    pub wait_timeout: u64,
}

#[derive(Debug, Subcommand)]
pub enum AnchorAction {
    /// Query the active store's on-chain anchor state.
    Status,
    /// Decode and print the embedded chain pointer of a module file.
    Inspect {
        /// Path to a compiled `.dig` module.
        module: std::path::PathBuf,
    },
}

#[derive(Debug, Args)]
#[command(
    after_help = "Pairs this device with your DIGHUb account (RFC-8628 style): prints a code,\nyou approve it in the web app, then the scoped session token is stored. The token\nproves a DIGHUb account; it has NO on-chain authority and never signs/anchors.\n\nEXAMPLES:\n  digstore login\n  digstore login --json"
)]
pub struct LoginArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore whoami\n  digstore whoami --json")]
pub struct WhoamiArgs {}

#[derive(Debug, Args)]
#[command(after_help = "EXAMPLES:\n  digstore logout")]
pub struct LogoutArgs {}

#[derive(Debug, Args)]
#[command(
    after_help = "Advances an EXISTING store from CI: reconstructs the store's local state \
(publisher key from --deploy-key / DIGSTORE_DEPLOY_KEY, current root from chain), stages \
--output-dir, then commits + pushes to DIGHUb as a new capsule. NEVER mints (no `init`).\n\nReads \
defaults from `dig.toml` in the current directory (store-id, output-dir, build-command, remote, \
network, wait-timeout). Flags/env override the file.\n\nCosts a uniform per-capsule price (paid in \
$DIG at the live rate) + an XCH fee per deploy (the \
on-chain root update), paid from the wallet seed (DIGSTORE_PASSPHRASE).\n\nTWO KINDS OF DEPLOY KEY \
(don't mix them up):\n  --deploy-key  the §21 PUBLISHER / HUB HEAD-PUSH key — lets DIGHUb ACCEPT \
the capsule. Reconstructs the store's local state. From `digstore deploy-key export`. Env: \
DIGSTORE_DEPLOY_KEY. REQUIRED.\n  --writer-key  (a DIFFERENT, optional key) the ON-CHAIN \
ROOT-ADVANCE authority — a revocable WRITER DELEGATE that advances the store's root WITHOUT the \
owner master seed (the hub Teams \"Deployer\" flow pre-authorizes it). Env: DIGSTORE_WRITER_KEY. \
Omit to advance the root with the wallet's owner seed.\n\n--if-changed skips the deploy (and the \
spend) when the built output matches the store's current on-chain version — safe to run on every \
push. --dry-run previews the resulting version + exact cost WITHOUT spending.\n\nEXAMPLES:\n  \
digstore deploy\n  digstore deploy --if-changed --message \"deploy ${GITHUB_SHA}\" --json\n  \
digstore deploy --dry-run"
)]
pub struct DeployArgs {
    /// The on-chain store id (64-hex) to advance. Overrides `dig.toml`'s `store-id`.
    #[arg(long = "store-id")]
    pub store_id: Option<String>,
    /// The built-output directory to publish. Overrides `dig.toml`'s `output-dir` (default `dist`).
    #[arg(long = "output-dir")]
    pub output_dir: Option<String>,
    /// A shell build command to run before staging (e.g. "npm ci && npm run build").
    /// Overrides `dig.toml`'s `build-command`. Skipped if neither is set.
    #[arg(long = "build-command")]
    pub build_command: Option<String>,
    /// The publisher deploy-key seed (64-hex), from `digstore deploy-key export`. Prefer the
    /// `DIGSTORE_DEPLOY_KEY` env var in CI so it is not visible in the process table. This is
    /// the §21 HEAD-PUSH key (lets DIGHUb accept the capsule) — distinct from `--writer-key`,
    /// which is the on-chain root-advance authority.
    #[arg(long = "deploy-key")]
    pub deploy_key: Option<String>,
    /// The on-chain WRITER DELEGATE key (64-hex seed) that advances the store's root WITHOUT the
    /// owner master seed (#17). The owner pre-authorized this writer (hub Teams "Deployer" /
    /// `updateStoreOwnership`); it can change ONLY the metadata root. Prefer `DIGSTORE_WRITER_KEY`
    /// in CI. Omitted => the wallet seed (owner) signs the root advance.
    #[arg(long = "writer-key")]
    pub writer_key: Option<String>,
    /// The 32-byte secret salt (64-hex) for a PRIVATE store. Public stores omit it. Prefer
    /// `DIGSTORE_STORE_SALT` in CI.
    #[arg(long = "salt")]
    pub salt: Option<String>,
    /// Commit message for the new capsule. Overrides `dig.toml`'s `message`.
    #[arg(long, short)]
    pub message: Option<String>,
    /// Seconds to wait for on-chain confirmation (default 300; 0 = single check, don't block).
    #[arg(long)]
    pub wait_timeout: Option<u64>,
    /// Chain network (default `mainnet`).
    #[arg(long)]
    pub network: Option<String>,
    /// $DIG to pay for this deploy, as a DIG amount (e.g. `100` or `87.5`; max 3 dp).
    /// Pricing is dynamic + USD-pegged — the hub computes the live amount and you pass
    /// it here; the CLI is deterministic and never fetches a price. Falls back to a
    /// protocol default if unset.
    /// Overrides `dig.toml`'s `dig-amount` / `DIGSTORE_DIG_AMOUNT`.
    #[arg(long = "dig-amount", value_name = "DIG", value_parser = parse_dig_amount)]
    pub dig_amount: Option<u64>,
    /// The `origin` remote to publish to (e.g. `dig://<store-id>` for the public DIGHUb, or a
    /// self-hosted node URL). Overrides `dig.toml`'s `remote`. Defaults to the public RPC.
    #[arg(long)]
    pub remote: Option<String>,
    /// Skip the deploy (and the $DIG + XCH spend) when the built output is identical to the
    /// store's current on-chain version — a no-op guard for CI that runs on every push.
    #[arg(long = "if-changed")]
    pub if_changed: bool,
    /// Preview the resulting version (root) + the exact DIG/XCH cost WITHOUT spending,
    /// anchoring, or publishing anything. Builds + stages so the previewed root is real.
    #[arg(long = "dry-run")]
    pub dry_run: bool,
    /// Build a PREVIEW capsule: run the real compile→verify→decrypt read path on your
    /// build, producing a local preview artifact (a `.dig` module) + a content-address,
    /// WITHOUT minting, WITHOUT advancing the on-chain root, and WITHOUT spending DIG.
    /// FREE — no chain, no wallet, no deploy key. The deploy-action serves this artifact
    /// to preview hosting. Mutually exclusive with `--dry-run`.
    #[arg(long, conflicts_with = "dry_run")]
    pub preview: bool,
    /// Where to write the `--preview` artifact (the compiled `.dig` module). Default:
    /// `<output-dir>/../.dig-preview/<root>.dig`.
    #[arg(long = "preview-out")]
    pub preview_out: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct DeployKeyArgs {
    #[command(subcommand)]
    pub action: DeployKeyAction,
}

#[derive(Debug, Subcommand)]
pub enum DeployKeyAction {
    /// Print (or write) the active store's publisher deploy key (64-hex seed) so it can be stored
    /// as a CI secret. This key authorizes publishing new capsules to DIGHUb; it has NO spend
    /// authority. Treat it like a credential.
    Export {
        /// Write the key to this file instead of stdout.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
}

#[derive(Debug, Args)]
#[command(
    after_help = "EXAMPLES:\n  digstore update\n  digstore update --check\n  digstore update --yes"
)]
pub struct UpdateArgs {
    /// Only report whether an update is available; never download.
    #[arg(long)]
    pub check: bool,
}

#[derive(Debug, Args)]
#[command(
    after_help = "One guided first-run to get you ready to PUBLISH. It walks three things, in \
order:\n  1. Seed — import an existing BIP-39 mnemonic or generate a new one. The seed signs every \
on-chain action and pays the per-capsule $DIG price + XCH per publish. It NEVER leaves your \
machine.\n  2. Funds — \
checks your wallet has enough $DIG + XCH for a publish, and points you at where to get more if \
not.\n  3. Login (optional) — a DIGHUb account so your published stores appear in your dashboard. \
The login only GATES the push to the public hub; it has NO on-chain authority. (Seed signs the \
chain; login gates the push — two different things.)\n\nEverything except generating a brand-new \
seed is safe to re-run. `digstore auth` is an alias.\n\nEXAMPLES:\n  digstore setup\n  digstore \
setup --generate\n  digstore setup --no-login --json"
)]
pub struct SetupArgs {
    /// Generate a brand-new seed instead of importing one (skips the import prompt).
    #[arg(long, conflicts_with = "import")]
    pub generate: bool,
    /// Import an existing seed (prompts for the mnemonic). The default when a seed
    /// is absent and neither flag is given in interactive mode.
    #[arg(long, conflicts_with = "generate")]
    pub import: bool,
    /// Skip the optional DIGHUb login step.
    #[arg(long)]
    pub no_login: bool,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Connects the CURRENT folder to a store you ALREADY published (from the hub or \
on-chain), so you can iterate + redeploy it from here. It writes a committable `dig.toml` pinning \
the store's id (and, when given a full URN, its remote), and registers `origin`. It does NOT mint, \
spend, download content, or need your seed — it just records where to publish.\n\nAfter linking, \
publish a new version with `digstore deploy` (which reconstructs the store from your deploy key).\n\n\
EXAMPLES:\n  digstore link 7e3a…  (a 64-hex store id)\n  digstore link urn:dig:chia:<storeID>\n  \
digstore link <storeID> --output-dir dist --remote dig://<storeID>"
)]
pub struct LinkArgs {
    /// The store to attach to: a 64-hex store id, or a `urn:dig:chia:<storeID>[:<root>]` URN.
    pub target: String,
    /// The built-output directory to publish (written to `dig.toml`; default `dist`).
    #[arg(long = "output-dir")]
    pub output_dir: Option<String>,
    /// The remote to publish to (written to `dig.toml`'s `remote`). Defaults to the
    /// public DIGHUb for the linked store when omitted.
    #[arg(long)]
    pub remote: Option<String>,
    /// Overwrite an existing `dig.toml` in this folder (otherwise linking refuses).
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Args)]
#[command(
    after_help = "Generates a shell completion script for `digstore` on stdout. Install it the way \
your shell expects, e.g.:\n  bash:        digstore completion bash > /etc/bash_completion.d/digstore\n  \
zsh:         digstore completion zsh  > \"${fpath[1]}/_digstore\"\n  fish:        digstore completion \
fish > ~/.config/fish/completions/digstore.fish\n  powershell:  digstore completion powershell >> \
$PROFILE\n\nEXAMPLES:\n  digstore completion bash\n  digstore completion zsh"
)]
pub struct CompletionArgs {
    /// The shell to generate completions for.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[test]
    fn parses_init() {
        let cli = Cli::try_parse_from(["digstore", "init"]).unwrap();
        assert!(matches!(cli.command, Command::Init(_)));
    }

    #[test]
    fn parses_new_template_and_dir() {
        let cli = Cli::try_parse_from(["digstore", "new", "static-site", "./site"]).unwrap();
        match cli.command {
            Command::New(a) => {
                assert_eq!(a.template, "static-site");
                assert_eq!(a.dir.unwrap().to_str().unwrap(), "./site");
                assert!(!a.force);
            }
            _ => panic!("expected new"),
        }
    }

    #[test]
    fn parses_new_list_flag() {
        let cli = Cli::try_parse_from(["digstore", "new", "x", "--list"]).unwrap();
        match cli.command {
            Command::New(a) => assert!(a.list),
            _ => panic!("expected new"),
        }
    }

    #[test]
    fn parses_dev_defaults() {
        let cli = Cli::try_parse_from(["digstore", "dev"]).unwrap();
        match cli.command {
            Command::Dev(a) => {
                assert_eq!(a.port, 4343);
                assert!(a.dir.is_none());
                assert!(!a.open);
            }
            _ => panic!("expected dev"),
        }
    }

    #[test]
    fn parses_dev_flags() {
        let cli =
            Cli::try_parse_from(["digstore", "dev", "--dir", "dist", "--port", "5000"]).unwrap();
        match cli.command {
            Command::Dev(a) => {
                assert_eq!(a.dir.as_deref(), Some("dist"));
                assert_eq!(a.port, 5000);
            }
            _ => panic!("expected dev"),
        }
    }

    #[test]
    fn parses_doctor() {
        let cli = Cli::try_parse_from(["digstore", "doctor"]).unwrap();
        assert!(matches!(cli.command, Command::Doctor(_)));
    }

    #[test]
    fn parses_commit_dry_run() {
        let cli = Cli::try_parse_from(["digstore", "commit", "--dry-run"]).unwrap();
        match cli.command {
            Command::Commit(c) => assert!(c.dry_run),
            _ => panic!("expected commit"),
        }
        // default off
        let cli = Cli::try_parse_from(["digstore", "commit"]).unwrap();
        match cli.command {
            Command::Commit(c) => assert!(!c.dry_run),
            _ => panic!("expected commit"),
        }
    }

    #[test]
    fn parses_deploy_if_changed_and_dry_run() {
        let cli = Cli::try_parse_from(["digstore", "deploy", "--if-changed"]).unwrap();
        match cli.command {
            Command::Deploy(d) => {
                assert!(d.if_changed);
                assert!(!d.dry_run);
            }
            _ => panic!("expected deploy"),
        }
        let cli = Cli::try_parse_from(["digstore", "deploy", "--dry-run"]).unwrap();
        match cli.command {
            Command::Deploy(d) => {
                assert!(d.dry_run);
                assert!(!d.if_changed);
            }
            _ => panic!("expected deploy"),
        }
        // defaults off
        let cli = Cli::try_parse_from(["digstore", "deploy"]).unwrap();
        match cli.command {
            Command::Deploy(d) => {
                assert!(!d.if_changed);
                assert!(!d.dry_run);
            }
            _ => panic!("expected deploy"),
        }
    }

    #[test]
    fn parses_deploy_preview() {
        // #18: `deploy --preview` builds a free preview capsule (no chain).
        let cli = Cli::try_parse_from(["digstore", "deploy", "--preview"]).unwrap();
        match cli.command {
            Command::Deploy(d) => {
                assert!(d.preview);
                assert!(!d.dry_run);
                assert!(d.preview_out.is_none());
            }
            _ => panic!("expected deploy"),
        }
        // --preview-out sets the artifact path.
        let cli =
            Cli::try_parse_from(["digstore", "deploy", "--preview", "--preview-out", "p.dig"])
                .unwrap();
        match cli.command {
            Command::Deploy(d) => {
                assert!(d.preview);
                assert_eq!(d.preview_out.unwrap().to_str().unwrap(), "p.dig");
            }
            _ => panic!("expected deploy"),
        }
        // --preview and --dry-run are mutually exclusive.
        assert!(Cli::try_parse_from(["digstore", "deploy", "--preview", "--dry-run"]).is_err());
    }

    #[test]
    fn parses_add_path() {
        let cli = Cli::try_parse_from(["digstore", "add", "file.txt"]).unwrap();
        match cli.command {
            Command::Add(a) => assert_eq!(a.paths[0].to_str().unwrap(), "file.txt"),
            _ => panic!("expected add"),
        }
    }

    #[test]
    fn parses_cat_urn() {
        let cli = Cli::try_parse_from(["digstore", "cat", "urn:dig:chia:abcd/readme"]).unwrap();
        match cli.command {
            Command::Cat(c) => assert_eq!(c.urn, "urn:dig:chia:abcd/readme"),
            _ => panic!("expected cat"),
        }
    }

    #[test]
    fn parses_remote_add_subcommand() {
        let cli =
            Cli::try_parse_from(["digstore", "remote", "add", "origin", "https://h/stores/x"])
                .unwrap();
        match cli.command {
            Command::Remote(r) => match r.action {
                RemoteAction::Add { name, url } => {
                    assert_eq!(name.as_deref(), Some("origin"));
                    assert_eq!(url.as_deref(), Some("https://h/stores/x"));
                }
                _ => panic!("expected remote add"),
            },
            _ => panic!("expected remote"),
        }
    }

    #[test]
    fn parses_update_check_flag() {
        let cli = Cli::try_parse_from(["digstore", "update", "--check"]).unwrap();
        assert!(!cli.yes); // the global --yes defaults off
        match cli.command {
            Command::Update(u) => {
                assert!(u.check);
            }
            _ => panic!("expected update"),
        }
    }

    #[test]
    fn parses_revoke_root_with_reason() {
        let cli = Cli::try_parse_from([
            "digstore",
            "revoke",
            "--root",
            "abcd",
            "--reason",
            "compromise",
        ])
        .unwrap();
        match cli.command {
            Command::Revoke(r) => {
                assert_eq!(r.root.as_deref(), Some("abcd"));
                assert!(!r.all);
                assert_eq!(r.reason, "compromise");
                assert_eq!(r.remote, "origin");
            }
            _ => panic!("expected revoke"),
        }
    }

    #[test]
    fn parses_revoke_all() {
        let cli = Cli::try_parse_from(["digstore", "revoke", "--all"]).unwrap();
        match cli.command {
            Command::Revoke(r) => {
                assert!(r.all);
                assert!(r.root.is_none());
            }
            _ => panic!("expected revoke"),
        }
    }

    #[test]
    fn revoke_rejects_root_and_all_together() {
        let err = Cli::try_parse_from(["digstore", "revoke", "--root", "ab", "--all"]);
        assert!(err.is_err(), "--root and --all are mutually exclusive");
    }

    #[test]
    fn parses_global_yes_flag() {
        // --yes is now a GLOBAL flag (works on any subcommand), not update-specific.
        let cli = Cli::try_parse_from(["digstore", "update", "--yes"]).unwrap();
        assert!(cli.yes);
        assert!(matches!(cli.command, Command::Update(_)));
    }

    #[test]
    fn parses_global_non_interactive_flag() {
        let cli = Cli::try_parse_from(["digstore", "--non-interactive", "status"]).unwrap();
        assert!(cli.non_interactive);
    }

    #[test]
    fn global_dig_dir_flag_before_subcommand() {
        let cli = Cli::try_parse_from(["digstore", "--dig-dir", "/tmp/d", "status"]).unwrap();
        assert_eq!(cli.dig_dir.unwrap().to_str().unwrap(), "/tmp/d");
    }

    #[test]
    fn global_json_flag_after_subcommand() {
        let cli = Cli::try_parse_from(["digstore", "status", "--json"]).unwrap();
        assert!(cli.json);
    }

    #[test]
    fn parses_balance() {
        let cli = Cli::try_parse_from(["digstore", "balance"]).unwrap();
        assert!(matches!(cli.command, Command::Balance(_)));
    }

    #[test]
    fn parses_setup_and_auth_alias() {
        let cli = Cli::try_parse_from(["digstore", "setup"]).unwrap();
        assert!(matches!(cli.command, Command::Setup(_)));
        // `auth` is a visible alias for `setup`.
        let cli = Cli::try_parse_from(["digstore", "auth"]).unwrap();
        assert!(matches!(cli.command, Command::Setup(_)));
    }

    #[test]
    fn parses_setup_flags() {
        let cli = Cli::try_parse_from(["digstore", "setup", "--generate", "--no-login"]).unwrap();
        match cli.command {
            Command::Setup(s) => {
                assert!(s.generate);
                assert!(s.no_login);
                assert!(!s.import);
            }
            _ => panic!("expected setup"),
        }
        // --generate and --import are mutually exclusive.
        assert!(Cli::try_parse_from(["digstore", "setup", "--generate", "--import"]).is_err());
    }

    #[test]
    fn parses_link_target_and_flags() {
        let cli = Cli::try_parse_from([
            "digstore",
            "link",
            "urn:dig:chia:abcd",
            "--output-dir",
            "dist",
        ])
        .unwrap();
        match cli.command {
            Command::Link(l) => {
                assert_eq!(l.target, "urn:dig:chia:abcd");
                assert_eq!(l.output_dir.as_deref(), Some("dist"));
                assert!(!l.force);
            }
            _ => panic!("expected link"),
        }
    }

    #[test]
    fn parses_completion_shell() {
        let cli = Cli::try_parse_from(["digstore", "completion", "bash"]).unwrap();
        match cli.command {
            Command::Completion(c) => assert_eq!(c.shell, clap_complete::Shell::Bash),
            _ => panic!("expected completion"),
        }
        // An unknown shell is rejected by the value_enum.
        assert!(Cli::try_parse_from(["digstore", "completion", "tcsh"]).is_err());
    }

    #[test]
    fn parses_commit_resubmit() {
        let cli = Cli::try_parse_from(["digstore", "commit", "--resubmit"]).unwrap();
        match cli.command {
            Command::Commit(c) => assert!(c.resubmit),
            _ => panic!("expected commit"),
        }
        // default is false
        let cli = Cli::try_parse_from(["digstore", "commit"]).unwrap();
        match cli.command {
            Command::Commit(c) => assert!(!c.resubmit),
            _ => panic!("expected commit"),
        }
    }

    #[test]
    fn parses_commit_push_flag() {
        let cli = Cli::try_parse_from(["digstore", "commit", "--push"]).unwrap();
        match cli.command {
            Command::Commit(c) => {
                assert!(c.push);
                assert!(!c.no_push);
            }
            _ => panic!("expected commit"),
        }
    }

    #[test]
    fn parses_commit_no_push_flag() {
        let cli = Cli::try_parse_from(["digstore", "commit", "--no-push"]).unwrap();
        match cli.command {
            Command::Commit(c) => {
                assert!(c.no_push);
                assert!(!c.push);
            }
            _ => panic!("expected commit"),
        }
    }

    #[test]
    fn commit_push_default_is_neither_flag() {
        // Default = ask when interactive, do nothing when not. Neither flag set.
        let cli = Cli::try_parse_from(["digstore", "commit"]).unwrap();
        match cli.command {
            Command::Commit(c) => {
                assert!(!c.push);
                assert!(!c.no_push);
            }
            _ => panic!("expected commit"),
        }
    }

    #[test]
    fn commit_push_and_no_push_are_mutually_exclusive() {
        let err = Cli::try_parse_from(["digstore", "commit", "--push", "--no-push"]);
        assert!(err.is_err(), "--push and --no-push are mutually exclusive");
    }

    #[test]
    fn parses_commit_writer_key() {
        // #17: `commit --writer-key <writer-seed>` advances the root with a writer delegate.
        // `--writer-key` matches `deploy`'s flag name + the Action; it is the on-chain
        // root-advance authority (NOT the §21 publisher --deploy-key).
        let cli =
            Cli::try_parse_from(["digstore", "commit", "--writer-key", &"ab".repeat(32)]).unwrap();
        match cli.command {
            Command::Commit(c) => {
                assert_eq!(c.writer_key.as_deref(), Some("ab".repeat(32).as_str()))
            }
            _ => panic!("expected commit"),
        }
        // Default: no writer key (owner-signed).
        let cli = Cli::try_parse_from(["digstore", "commit"]).unwrap();
        match cli.command {
            Command::Commit(c) => assert!(c.writer_key.is_none()),
            _ => panic!("expected commit"),
        }
    }

    #[test]
    fn commit_deploy_key_is_hidden_deprecated_alias_for_writer_key() {
        // Back-compat: `--deploy-key` on `commit` historically meant the WRITER key.
        // It remains a (hidden, deprecated) alias of `--writer-key` so old scripts work.
        let cli =
            Cli::try_parse_from(["digstore", "commit", "--deploy-key", &"cd".repeat(32)]).unwrap();
        match cli.command {
            Command::Commit(c) => {
                assert_eq!(c.writer_key.as_deref(), Some("cd".repeat(32).as_str()))
            }
            _ => panic!("expected commit"),
        }
        // The canonical OPTION shown in the help options section is `--writer-key`; the
        // `--deploy-key` alias is a plain (hidden) `alias`, so clap does not list it as
        // its own option entry. We assert the canonical flag is registered with that id.
        let commit = Cli::command().find_subcommand("commit").unwrap().clone();
        let arg = commit
            .get_arguments()
            .find(|a: &&clap::Arg| a.get_id() == "writer_key")
            .expect("writer_key arg exists");
        assert_eq!(arg.get_long(), Some("writer-key"));
    }

    #[test]
    fn parses_anchor_resume() {
        let cli = Cli::try_parse_from(["digstore", "anchor"]).unwrap();
        match cli.command {
            Command::Anchor(a) => {
                assert!(a.action.is_none());
                assert_eq!(a.wait_timeout, 300);
            }
            _ => panic!("expected anchor"),
        }
    }

    #[test]
    fn parses_anchor_status() {
        let cli = Cli::try_parse_from(["digstore", "anchor", "status"]).unwrap();
        match cli.command {
            Command::Anchor(a) => assert!(matches!(a.action, Some(AnchorAction::Status))),
            _ => panic!("expected anchor status"),
        }
    }

    #[test]
    fn parses_anchor_inspect() {
        let cli = Cli::try_parse_from(["digstore", "anchor", "inspect", "x.dig"]).unwrap();
        match cli.command {
            Command::Anchor(a) => match a.action {
                Some(AnchorAction::Inspect { module }) => {
                    assert_eq!(module.to_str().unwrap(), "x.dig")
                }
                _ => panic!("expected inspect"),
            },
            _ => panic!("expected anchor"),
        }
    }

    #[test]
    fn parses_anchor_wait_timeout() {
        let cli = Cli::try_parse_from(["digstore", "anchor", "--wait-timeout", "0"]).unwrap();
        match cli.command {
            Command::Anchor(a) => {
                assert!(a.action.is_none());
                assert_eq!(a.wait_timeout, 0);
            }
            _ => panic!("expected anchor"),
        }
    }

    #[test]
    fn private_salt_flag_on_cat() {
        let cli = Cli::try_parse_from([
            "digstore",
            "cat",
            "urn:dig:chia:abcd/r",
            "--salt",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ])
        .unwrap();
        match cli.command {
            Command::Cat(c) => assert!(c.salt.is_some()),
            _ => panic!("expected cat"),
        }
    }

    // --- Vocabulary (SYSTEM.md "Canonical terminology"): user-facing copy says
    // `store`/`capsule`, never "project". `--project`/`projects` remain ONLY as
    // HIDDEN, deprecated back-compat aliases — they must keep parsing, but must NOT
    // appear in `--help`. ---

    #[test]
    fn projects_is_alias_for_stores() {
        let cli = Cli::try_parse_from(["digstore", "projects"]).unwrap();
        assert!(matches!(cli.command, Command::Stores(_)));
    }

    #[test]
    fn stores_command_still_works() {
        // backward-compat guard: the original command name is unchanged.
        let cli = Cli::try_parse_from(["digstore", "stores"]).unwrap();
        assert!(matches!(cli.command, Command::Stores(_)));
    }

    #[test]
    fn project_flag_is_alias_for_store() {
        let cli = Cli::try_parse_from(["digstore", "--project", "site", "status"]).unwrap();
        assert_eq!(cli.store_name.as_deref(), Some("site"));
    }

    /// The `projects` command alias and the `--project` global flag alias are HIDDEN:
    /// they still parse, but neither the word "project" nor the alias is advertised in
    /// help. (Vocabulary purge — store/capsule are the only user-facing terms.)
    #[test]
    fn project_aliases_are_hidden_from_help() {
        let cmd = Cli::command();
        // `stores` advertises no visible alias (the `projects` alias is hidden).
        let stores = cmd.find_subcommand("stores").unwrap();
        assert!(
            stores.get_visible_aliases().next().is_none(),
            "the `projects` alias must be hidden"
        );
        // The global `--store` flag advertises no visible alias either (the
        // `--project` alias is a plain hidden `alias`).
        let store_flag = cmd
            .get_arguments()
            .find(|a: &&clap::Arg| a.get_id() == "store_name")
            .expect("store flag exists");
        let visible = store_flag
            .get_visible_aliases()
            .map(|v| v.contains(&"project"))
            .unwrap_or(false);
        assert!(!visible, "the `--project` flag alias must be hidden");
    }

    #[test]
    fn store_flag_still_works() {
        // backward-compat guard: the original --store flag is unchanged.
        let cli = Cli::try_parse_from(["digstore", "--store", "site", "status"]).unwrap();
        assert_eq!(cli.store_name.as_deref(), Some("site"));
    }

    // --- Wave-B asset CLI (#35) parse guards ---

    #[test]
    fn parses_nft_mint() {
        let cli = Cli::try_parse_from([
            "digstore",
            "nft",
            "mint",
            "--art",
            "a.png",
            "--name",
            "DIG Punk #1",
            "--royalty",
            "300",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Nft(NftArgs {
                action: NftAction::Mint(m),
            }) => {
                assert_eq!(m.art.to_str().unwrap(), "a.png");
                assert_eq!(m.name, "DIG Punk #1");
                assert_eq!(m.royalty, 300);
                assert!(m.dry_run);
                assert!(m.did.is_none());
            }
            _ => panic!("expected nft mint"),
        }
    }

    #[test]
    fn parses_nft_transfer_and_list() {
        let cli = Cli::try_parse_from([
            "digstore", "nft", "transfer", "--nft", "abcd", "--to", "xch1z",
        ])
        .unwrap();
        match cli.command {
            Command::Nft(NftArgs {
                action: NftAction::Transfer(t),
            }) => {
                assert_eq!(t.nft, "abcd");
                assert_eq!(t.to, "xch1z");
            }
            _ => panic!("expected nft transfer"),
        }
        let cli = Cli::try_parse_from(["digstore", "nft", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Nft(NftArgs {
                action: NftAction::List(_)
            })
        ));
    }

    #[test]
    fn parses_nft_bulk() {
        let cli = Cli::try_parse_from([
            "digstore",
            "nft",
            "bulk",
            "--manifest",
            "items.json",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Nft(NftArgs {
                action: NftAction::Bulk(b),
            }) => {
                assert_eq!(b.manifest.to_str().unwrap(), "items.json");
                assert!(b.dry_run);
            }
            _ => panic!("expected nft bulk"),
        }
    }

    #[test]
    fn parses_collection_create_and_mint() {
        let cli = Cli::try_parse_from([
            "digstore",
            "collection",
            "create",
            "--name",
            "DIG Punks",
            "--royalty",
            "300",
        ])
        .unwrap();
        match cli.command {
            Command::Collection(CollectionArgs {
                action: CollectionAction::Create(c),
            }) => {
                assert_eq!(c.name, "DIG Punks");
                assert_eq!(c.royalty, 300);
                assert!(c.id.is_none());
            }
            _ => panic!("expected collection create"),
        }
        let cli = Cli::try_parse_from([
            "digstore",
            "collection",
            "mint",
            "--collection",
            "c.json",
            "--manifest",
            "i.json",
            "--did",
            "abcd",
        ])
        .unwrap();
        match cli.command {
            Command::Collection(CollectionArgs {
                action: CollectionAction::Mint(m),
            }) => {
                assert_eq!(m.collection.to_str().unwrap(), "c.json");
                assert_eq!(m.did, "abcd");
            }
            _ => panic!("expected collection mint"),
        }
    }

    #[test]
    fn parses_collection_show_and_list() {
        // #39: collection reads off coinset.
        let cli = Cli::try_parse_from(["digstore", "collection", "show", "--did", "abcd"]).unwrap();
        match cli.command {
            Command::Collection(CollectionArgs {
                action: CollectionAction::Show(s),
            }) => assert_eq!(s.did, "abcd"),
            _ => panic!("expected collection show"),
        }
        let cli = Cli::try_parse_from(["digstore", "collection", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Collection(CollectionArgs {
                action: CollectionAction::List(_)
            })
        ));
    }

    #[test]
    fn parses_did_create() {
        let cli = Cli::try_parse_from(["digstore", "did", "create", "--dry-run"]).unwrap();
        match cli.command {
            Command::Did(DidArgs {
                action: DidAction::Create(c),
            }) => assert!(c.dry_run),
            _ => panic!("expected did create"),
        }
    }

    #[test]
    fn parses_offer_make_take_show() {
        let cli = Cli::try_parse_from([
            "digstore",
            "offer",
            "make",
            "--offer",
            "1000xch",
            "--request",
            "100dig",
        ])
        .unwrap();
        match cli.command {
            Command::Offer(OfferArgs {
                action: OfferAction::Make(m),
            }) => {
                assert_eq!(m.offer, vec!["1000xch".to_string()]);
                assert_eq!(m.request, vec!["100dig".to_string()]);
            }
            _ => panic!("expected offer make"),
        }
        let cli = Cli::try_parse_from([
            "digstore",
            "offer",
            "take",
            "--offer",
            "offer1xyz",
            "--dry-run",
        ])
        .unwrap();
        match cli.command {
            Command::Offer(OfferArgs {
                action: OfferAction::Take(t),
            }) => {
                assert_eq!(t.offer, "offer1xyz");
                assert!(t.dry_run);
            }
            _ => panic!("expected offer take"),
        }
        let cli =
            Cli::try_parse_from(["digstore", "offer", "show", "--offer", "offer1xyz"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Offer(OfferArgs {
                action: OfferAction::Show(_)
            })
        ));
    }
}
