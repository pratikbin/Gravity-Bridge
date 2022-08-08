package v3

import (
	"github.com/cosmos/cosmos-sdk/codec"
	"github.com/cosmos/cosmos-sdk/store/prefix"
	storetypes "github.com/cosmos/cosmos-sdk/store/types"
	sdk "github.com/cosmos/cosmos-sdk/types"
	"github.com/tendermint/tendermint/libs/log"

	"github.com/Gravity-Bridge/Gravity-Bridge/module/x/gravity/types"
)

// MigrateStore performs in-place store migrations from v2 to v3. The migration
// includes:
//
// - Migrate all attestations to their correct new keys
func MigrateStore(ctx sdk.Context, storeKey storetypes.StoreKey, cdc codec.BinaryCodec) error {
	ctx.Logger().Info("enterupgradename Upgrade: Beginning the migrations for the gravity module")
	store := ctx.KVStore(storeKey)

	convertAttestationKey := getAttestationConverter(ctx.Logger())
	// Migrate all stored attestations by iterating over everything stored under the OracleAttestationKey
	ctx.Logger().Info("enterupgradename Upgrade: Beginning Attestation Upgrade")
	if err := migrateKeysFromValues(store, cdc, types.OracleAttestationKey, convertAttestationKey); err != nil {
		return err
	}

	ctx.Logger().Info("enterupgradename Upgrade: Finished the migrations for the gravity module successfully!")
	return nil
}

// Iterates over every value stored under keyPrefix, computes the new key using getNewKey,
// then stores the value in the new key before deleting the old key
func migrateKeysFromValues(store sdk.KVStore, cdc codec.BinaryCodec, keyPrefix []byte, getNewKey func([]byte, codec.BinaryCodec, []byte) ([]byte, error)) error {
	oldStore := prefix.NewStore(store, keyPrefix)
	oldStoreIter := oldStore.Iterator(nil, nil)
	defer oldStoreIter.Close()

	for ; oldStoreIter.Valid(); oldStoreIter.Next() {
		// Set new key on store. Values don't change.
		oldKey := oldStoreIter.Key()
		value := oldStoreIter.Value()
		newKey, err := getNewKey(value, cdc, oldKey)
		if err != nil {
			return err
		}
		store.Set(newKey, value)
		oldStore.Delete(oldKey)
	}
	return nil
}

// Creates a closure with the current logger for the attestation key conversion function
func getAttestationConverter(logger log.Logger) func([]byte, codec.BinaryCodec, []byte) ([]byte, error) {
	// Unmarshal the old Attestation, unpack its claim, recompute the key using the new ClaimHash
	return func(oldValue []byte, cdc codec.BinaryCodec, oldKey []byte) ([]byte, error) {
		var att types.Attestation
		cdc.MustUnmarshal(oldValue, &att)
		claim, err := unpackAttestationClaim(&att, cdc)
		if err != nil {
			return nil, err
		}
		hash, err := claim.ClaimHash()
		if err != nil {
			return nil, err
		}

		newKey, err := types.GetAttestationKey(claim.GetEventNonce(), hash), nil

		logger.Info("enterupgradename Upgrade: Migrating attestation", "type", claim.GetType(), "nonce", claim.GetEventNonce(), "eth-block-height", claim.GetEthBlockHeight())

		return newKey, err
	}
}

// Unpacks the contained EthereumClaim
func unpackAttestationClaim(att *types.Attestation, cdc codec.BinaryCodec) (types.EthereumClaim, error) {
	var msg types.EthereumClaim
	err := cdc.UnpackAny(att.Claim, &msg)
	if err != nil {
		return nil, err
	} else {
		return msg, nil
	}
}
