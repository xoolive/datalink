# Decode standalone frames and payloads

```sh
# Hex-encoded ACARS frame bytes
datalink decode acars --direction downlink '<hex>'

# Hex-encoded AVLC frame including FCS
datalink decode avlc '<hex>'

# ARINC 622 application text envelope; dispatches ADS-C, CPDLC, DIS, or raw IMI payloads
datalink decode arinc622 --direction downlink '/ATSU.ADS....'
datalink decode arinc622 --direction uplink '/ATSU.AT1....'

# Strict ADS-C ARINC 622 envelope, including ADS-C DIS disconnect messages
datalink decode adsc --direction downlink '/ATSU.ADS....'
datalink decode adsc --direction uplink '/ATSU.ADS....'

# Strict CPDLC ARINC 622 envelope or control message
datalink decode cpdlc --direction uplink '/ATSU.AT1....'
datalink decode cpdlc --direction uplink '/ATSU.CR1....'
```
