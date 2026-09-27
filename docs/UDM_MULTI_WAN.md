# UDM Multi-WAN setup for StorDown

The physical topology can include a normal Ethernet switch between the Windows PC and the UDM.

```text
Windows PC
  |- NIC 1 --\
  |          +--> TP-Link switch --> UDM Pro
  |- NIC 2 --/
                           |- WAN1
                           \- WAN2
```

A 1 GbE switch/uplink is enough to test two ~400 Mbps upload/download paths together, although it leaves limited headroom above roughly 800 Mbps of aggregate traffic.

## Recommended setup

1. Keep NIC teaming/bridging disabled in Windows.
2. Give each NIC its own stable IPv4 address, for example:
   - NIC 1: `192.168.30.101`
   - NIC 2: `192.168.30.102`
3. In UniFi, create one policy-based route for NIC 1 / its source IP to WAN1.
4. Create another policy-based route for NIC 2 / its source IP to WAN2.
5. Keep both policies active while testing StorDown.
6. Start with 8 HTTP segments and both bind IPs.
7. Watch WAN1 and WAN2 traffic graphs while downloading a large file that supports HTTP Range.

The example IPs are placeholders. Use addresses that match the actual LAN.

## Why this works

StorDown binds different HTTP workers to different source IPs. The UDM can then apply a deterministic WAN policy to each source instead of hashing all flows automatically.

## Upgrade point

If aggregate Internet capacity grows beyond the practical limit of Gigabit Ethernet, upgrade the PC/switch/UDM uplink path to 2.5 GbE or faster before expecting more than ~1 Gbps total throughput.
