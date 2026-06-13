### Decode standalone frames

```sh
# Hex-encoded ACARS frame bytes
datalink decode acars --direction downlink '<hex>'

# Hex-encoded AVLC frame including FCS
datalink decode avlc '<hex>'

# ADS-C application text payload
datalink decode adsc '/ATSU.ADS....'
```
