# Tools

## emem_ask_place

All-in-one tool for most users. Takes a place and a question, returns signed geospatial facts.

### Input

```json
{
  "place": "South Mumbai",
  "question": "What signed geospatial facts are available here?",
  "include_image": false
}
```

### Endpoint

```
POST https://emem.dev/v1/ask
```

### Returns

Signed facts with CIDs, caveats, and receipts.

---

## emem_locate_place

Resolves a place name to coordinates and available data.

### Input

```json
{
  "place": "Helsinki Airport"
}
```

### Returns

- cell64
- Coordinates (lat, lon)
- Available bands (elevation, surface water, vegetation, etc.)

---

## emem_recall_facts

Recalls signed geospatial facts for a specific cell and bands.

### Input

```json
{
  "cell": "...",
  "bands": ["copdem30m.elevation_mean", "surface_water.recurrence"]
}
```

### Returns

Signed facts with values, CIDs, and receipts for each requested band.

---

## emem_get_receipt

Returns a verifiable receipt for a specific fact CID.

### Input

```json
{
  "cid": "bafy..."
}
```

### Returns

- ed25519 signature
- Signer public key
- Timestamp
- Fact payload hash
