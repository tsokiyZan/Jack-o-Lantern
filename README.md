# Pumpkin, tsokiyZan's fork

> [!WARNING]
> This is a personal fork of [Pumpkin](https://github.com/Pumpkin-MC/Pumpkin), pushed toward
> vanilla parity as fast as possible. Almost all of the code added here is written by an AI
> coding agent (Claude Code) and playtested by one person. Expect bugs. If you need something
> dependable, use upstream.

## What this fork does

- Merges open upstream pull requests early, after playing them on a real 26.3 client.
- Ports missing behaviour from the decompiled vanilla server, method by method.
- Fixes whatever turns up while playing.
- Merges upstream `master` regularly, so it stays a superset of upstream.

## What is different from upstream

| Change | Source |
| :-- | :-- |
| Piglins and piglin brutes run on the brain system | Upstream PR 3630 by wiflow, merged here; barter plugin event restored |
| Fish and squid swim instead of walking | Upstream PR 3718 by KBDL, merged here |
| Squid and glow squid squirt ink when hurt | Ported here from `Squid.spawnInk` |
| Cod, salmon and tropical fish school | Ported here from `AbstractSchoolingFish` and `FollowFlockLeaderGoal` |
| Pufferfish puff up, sting and poison | Ported here from `Pufferfish` |
| Eyeblossoms no longer disconnect nearby players | Fixed here; the trail particle now carries its options |
| All living entities have an air supply and can drown | Upstream PR by colestebbens, merged here; turning off `drowning_damage` still spares players |
| Fire burns out, and clients see it go out | Fixed here |
| Scheduled block ticks survive chunk unload and load, and only run in ticking chunks | Fixed here |
| Pale hanging moss needs support | Ported here |
| `/forceload` actually loads the chunks | Fixed here |
| Dolphins: full vanilla AI, including treasure hunting, item play, babies, following boats and swimming with players | Ported here from `Dolphin` |
| Mob despawning and `noActionTime` follow vanilla; nothing despawns with no player online | Ported here from `Mob.checkDespawn` |
| Water pathfinding costs: land mobs avoid water as in vanilla, and swimmers, blazes, striders, bees, frogs and sniffers use their own costs | Ported here from each mob's constructor |
| The path search reaches exactly as far as vanilla's, and mobs keep their first path to a still target | Ported here from `PathFinder` and `PathNavigation.createPath` |
| Zombies, husks, zombie villagers and drowned use vanilla's goal lists: zombies sink, target wandering traders and only baby turtles; drowned leave land targets alone by day | Ported here from `Zombie` and `Drowned` |
| Melee mobs walk right up to their target | Ported here; `MeleeAttackGoal` paths with reach 0 |
| Babies have vanilla hitbox sizes, and the `scale` attribute resizes hitboxes | Ported here from `getDimensions` and each mob's `BABY_DIMENSIONS` |
| Mobs walk at vanilla speed, and goal speeds (panicking, tempting) match | Fixed here: input decay order and `getSpeed` |

## Issues

Open issues from upstream are mirrored into this repository, with `[upstream N]` at the end
of the title and a link back to the original. An issue is closed here when it is fixed in
this fork, which says nothing about its state upstream. Please report upstream bugs upstream.

## Running a debug build

Debug builds currently overflow the stack during chunk generation unless threads get a
larger stack:

```bash
RUST_MIN_STACK=33554432 cargo run -p pumpkin
```

## Credit and license

Pumpkin is the work of the [Pumpkin-MC contributors](https://github.com/Pumpkin-MC/Pumpkin/graphs/contributors).
This fork is not affiliated with or endorsed by them. Everything here stays under the same
license as upstream, GPL-3.0 for the server.

The rest of this file is upstream's README, unchanged.

---

<div align="center">

# Pumpkin

![CI](https://github.com/Pumpkin-MC/Pumpkin/actions/workflows/rust.yml/badge.svg)
[![Discord](https://img.shields.io/discord/1268592337445978193.svg?label=&logo=discord&logoColor=ffffff&color=7389D8&labelColor=6A7EC2)](https://discord.gg/wT8XjrjKkf)
[![License: GPL](https://img.shields.io/badge/License-GPLv3-yellow.svg)](https://opensource.org/licenses/gpl-3-0)

</div>

[Pumpkin](https://pumpkinmc.org/) is a Minecraft server built entirely in Rust, offering a fast, efficient,
and customizable experience. It prioritizes performance and player enjoyment while adhering to the core mechanics of the game.
<div align="center">

![Pumpkin Chunk Loading](./assets/pumpkin-chunk-loading.webp)

</div>

## Goals

- **Performance**: Leveraging multi-threading for maximum speed and efficiency.
- **Compatibility**: Supports the latest Java & Bedrock Minecraft server version while adhering to Vanilla game mechanics.
- **Security**: Prioritizes security by preventing known security exploits.
- **Flexibility**: Highly configurable, with the ability to disable unnecessary features.
- **Extensibility**: Provides a foundation for plugin development.

> [!IMPORTANT]
> Pumpkin is currently under heavy development.
>
> [See what needs to be done before the 1.0.0 Release](https://github.com/Pumpkin-MC/Pumpkin/issues/449)

## Features

- [x] Configuration (toml)
- [Tracking: Protocol](https://github.com/Pumpkin-MC/Pumpkin/issues/1401)
  - [x] Server Status/Ping
  - [x] Encryption
  - [x] Packet Compression
  - [x] Java Edition
  - [x] Bedrock Edition (W.I.P)
  - ...
- [Tracking: World](https://github.com/Pumpkin-MC/Pumpkin/issues/1403)
  - [x] Player Tab-list
  - [x] Scoreboard
  - [x] World Loading
  - [x] World Time
  - [x] World Borders
  - [x] World Saving
  - [x] Lighting
  - [x] Entity Spawning
  - [x] Bossbar
  - [x] Chunk Loading (Vanilla, Linear, Pump)
  - [Chunk Generation](https://github.com/Pumpkin-MC/Pumpkin/issues/36)
  - [x] Chunk Saving (Vanilla, Linear, Pump)
  - [Redstone](https://github.com/Pumpkin-MC/Pumpkin/issues/1402)
  - [x] Liquid Physics
  - ...
- [Tracking: Player](https://github.com/Pumpkin-MC/Pumpkin/issues/1405)
  - [x] Skins
  - [x] Teleport
  - [x] Movement
  - [x] Animation
  - [x] Inventory
  - [Combat](https://github.com/Pumpkin-MC/Pumpkin/issues/1404)
  - [x] Experience
  - [x] Hunger
  - [X] Off Hand
  - [X] Advancements (W.I.P)
  - [x] Eating
  - ...
- Entities
  - [x] Non-Living (Minecart, Eggs...) (W.I.P)
  - [x] Entity Effects
  - [x] Players
  - [x] Mobs (W.I.P)
  - [x] Animals (W.I.P)
  - [Entity AI](https://github.com/Pumpkin-MC/Pumpkin/issues/1406)
  - [x] Boss (W.I.P)
  - [x] Villagers (W.I.P)
  - [X] Entity Saving
- Server
  - [Plugins](https://github.com/Pumpkin-MC/Pumpkin/issues/1407)
  - [x] Query
  - [x] [Minecraft Server Management Protocol (MSMP)](https://minecraft.wiki/w/Minecraft_Server_Management_Protocol)
  - [x] Inventories
  - [x] Particles
  - [x] Chat
  - [Commands](https://github.com/Pumpkin-MC/Pumpkin/issues/15)
  - [x] Permissions
  - [x] Translations
- Proxy
  - [x] [BungeeCord](https://github.com/SpigotMC/BungeeCord)
  - [x] [BungeeGuard](https://github.com/lucko/BungeeGuard)
  - [x] [Velocity](https://github.com/PaperMC/Velocity)

<!-- Check out our [Github Project](https://github.com/orgs/Pumpkin-MC/projects/3) to see current progress. -->

## How to run

See our [Quick Start](https://docs.pumpkinmc.org/#quick-start) guide to get Pumpkin running.

## Contributions

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md)

## Docs

Pumpkin's documentation can be found at <https://pumpkinmc.org/>

## Communication

Consider joining [our Discord server](https://discord.gg/wT8XjrjKkf) to stay up-to-date on events, updates, and connect with other members.

## Funding

If you want to fund me and help the project, check out the [Donation Page](https://pumpkinmc.org/donate/).

## License & Attribution

* **Pumpkin Server**: Licensed under the [GNU General Public License v3.0 (GPLv3)](LICENSE).
* **Plugin API (`pumpkin-plugin-api` & `pumpkin-plugin-wit`)**: Dual-licensed under [MIT](crates/pumpkin-plugin-api/LICENSE-MIT) OR [Apache-2.0](crates/pumpkin-plugin-api/LICENSE-APACHE) for maximum flexibility when writing plugins.
* **Third-Party Assets & Data**: Bedrock mappings, protocol conversion data, and Minecraft assets are subject to their respective licenses and attribution terms. See [assets/NOTICE.md](assets/NOTICE.md) for full details.
