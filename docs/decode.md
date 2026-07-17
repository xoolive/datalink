# Decode standalone frames

```sh
# Hex-encoded ACARS frame bytes
datalink decode acars --direction downlink '<hex>'

# Hex-encoded AVLC frame including FCS
datalink decode avlc '<hex>'

# ADS-C application text payload (direction is required for uplink contracts)
datalink decode adsc --direction downlink '/ATSU.ADS....'
datalink decode adsc --direction uplink '/ATSU.ADS....'
```
