# Datastore Coin Requirements

## Overview
Datastore coins are CAT (Colored Coins) on the Chia blockchain that represent storage commitments and require DIG token collateral. They provide a decentralized mechanism for ensuring data availability and incentivizing storage providers.

## Core Requirements

### 1. Token Standard
- **MUST** use Chia CAT (Colored Coin) standard
- **MUST** use DIG tokens for collateral (not XCH or mojos)
- **MUST** support standard CAT operations (transfer, spend, etc.)

### 2. Collateral System
- **MUST** require DIG token collateral based on datastore size
- **Base Rate**: 0.1 DIG tokens per GB
- **Large Datastore Multiplier**: 1.5x for datastores over 1GB
- **Precision**: 8 decimal places for DIG token amounts
- **Grace Period**: 30 days before collateral can be reclaimed

### 3. Coin Lifecycle
- **States**: Pending → Active → Expired → Spent
- **Creation**: Lock collateral and record datastore metadata
- **Minting**: Deploy to blockchain with transaction ID
- **Transfer**: Change ownership while maintaining collateral
- **Spending**: Release collateral after grace period

### 4. Metadata Requirements
Each coin MUST store:
- Datastore ID (derived from root hash)
- Root hash of the datastore
- Size in bytes
- Collateral amount in DIG tokens
- Owner address
- Host address (optional)
- Creation timestamp
- Expiration timestamp (optional)
- Blockchain transaction ID (when minted)
- Block height (when minted)

### 5. Security Requirements
- **MUST** verify wallet ownership before operations
- **MUST** check DIG token balance before creating coins
- **MUST** validate all state transitions
- **MUST** persist coin data securely
- **MUST** handle blockchain errors gracefully

### 6. CLI Requirements
Commands MUST include:
- `coin create` - Create new datastore coin
- `coin mint` - Mint coin on blockchain
- `coin list` - List coins with filtering options
- `coin info` - Show detailed coin information
- `coin transfer` - Transfer ownership
- `coin spend` - Spend coin and release collateral
- `coin stats` - Show aggregate statistics
- `coin collateral` - Calculate collateral requirements

### 7. Integration Requirements
- **MUST** integrate with existing Store operations
- **MUST** support configuration via TOML files
- **MUST** provide extension traits for Store
- **MUST** support both mainnet and testnet
- **MAY** auto-create coins on commit (configurable)

### 8. Testing Requirements
- **MUST** have unit tests for all components
- **MUST** have integration tests for workflows
- **MUST** test error conditions
- **MUST** test state transitions
- **MUST** test collateral calculations
- **MUST** test persistence and recovery

## Implementation Status
All requirements have been implemented in the datastore_coin module.