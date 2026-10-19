# Jack o'Lantern

Jack o'Lantern is tsokiyZan's fork of Pumpkin, a Minecraft server written in Rust.

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
| Ground mobs steer through vanilla's `MoveControl`; navigation follows paths like `PathNavigation` and bodies turn with `BodyRotationControl` | Ported here |
| Path search matches vanilla: neighbour order, heap ties, per-mob costs, door and campfire states, real collision shapes | Ported here from `PathFinder` and the node evaluators |
| Mob movement physics in vanilla's float maths, with a bit-exact `Mth.atan2` | Ported here from `LivingEntity.travel` and `Mth` |
| Targeting scales with visibility, line of sight uses colliders, revenge memory expires | Ported here from `TargetingConditions` and `LivingEntity` |
| Vanilla goal lists for spiders (leaps, armadillo avoidance), wolves (sitting), cats and ocelots (pouncing), illagers, ravagers, witches, vexes, snow golems, silverfish, slimes, ghasts, zombified piglins, farm animals | Ported here from each mob's `registerGoals` |
| Per-mob AI: witches heal raiders, vindicators break doors (and Johnny attacks everything), evokers and illusioners cast spells, vexes charge, guardians fire lasers, drowned swim and throw tridents, bees use hives and pollinate, phantoms run the circling and swooping AI, turtles go home and lay eggs, striders seek lava, llamas form caravans, shulkers defend their team, silverfish wake their friends, rabbits hop and raid carrot farms | Ported here from each mob's class and goals |
| Foxes, pandas, ghasts and happy ghasts (ghastlings on the `HappyGhastAi` brain) behave like vanilla; parrots can be tamed and sit; horses are tamed by riding them | Ported here from `Fox`, `Panda`, `Ghast`, `HappyGhast`, `Parrot` and `AbstractHorse` |
| Entity basics follow 26.3: collisions, the damage cooldown and `hurtTime`, daylight burning, the armor attribute, attribute modifier order, `NoGravity`, and saving leashes, attributes, homes and mob flags | Fixed here |
| Entity, worldgen and velocity maths use vanilla's float operation order (no fused multiply-add); player movement is sent with vanilla's position delta rounding | Fixed here |
| Every item data component encodes, decodes and hashes like vanilla (goat horns, paintings, trims and banners no longer disconnect) | Ported here |
| Dappled forest trees, vegetation patches and feature order generate like vanilla | Fixed here |
| Structure starts are computed once and biome-checked before their pieces are built (about 25% faster chunk loading) | Fixed here |
| Explosions follow vanilla's ray casting, block resistance, damage and knockback; block clipping matches `VoxelShape.clip`; wind charges burst where vanilla puts them | Ported here from `ServerExplosion` and `ServerLevel.explode` |
| Shoves sync like 26.3: being pushed by mobs no longer launches players, real hits still knock back | Fixed here |
| Damage events name the causing entity and the direct one the right way round; arrows hit as the direct source with their shooter responsible | Fixed here |
| Kill credit remembers a player's hit for 100 ticks (it used the day time, which stands still with the day cycle off) | Ported here from `lastHurtByPlayerMemoryTime` |
| Mob loot and dropped equipment spawn at the mob, like `spawnAtLocation` | Fixed here |
| Effects that trigger on death (oozing, weaving, wind charged) fire when the body is removed, like vanilla; effect clouds tick like `AreaEffectCloud` and creepers leave one | Ported here |
| Water potions hurt endermen, blazes and other water-sensitive mobs, put out burning entities and rehydrate axolotls | Ported here from `AbstractThrownPotion` |
| Slimes and magma cubes: vanilla `AbstractCubeMob` (sizes reach clients, splitting after the death animation, magma jumps and lava jumps) | Ported here |
| Squid and glow squid: vanilla goals, babies, dark ticks; stranded fish flop like vanilla | Ported here from `Squid`, `GlowSquid` and `AbstractFish` |
| Skeletons, strays, bogged, parched and wither skeletons: vanilla bow goal, tipped arrows per type, stray conversion in powder snow, bogged shearing, wither effect on hit | Ported here from `AbstractSkeleton` and subclasses |
| Zombie family: spear and move-through-village goals, hard-mode reinforcements, drowning conversions, villager infection, chicken jockeys and leaders; zombified piglins share the zombie base | Ported here from `Zombie`, `Husk`, `Drowned`, `ZombieVillager` and `ZombifiedPiglin` |
| Ravagers are stunned by shields and roar; shields block melee hits | Ported here from `Ravager` |
| Endermen teleport by day, drop their carried block and dodge projectiles; bats fly and roost like vanilla; giants have no AI | Ported here |
| Llamas, iron golems (village defence), polar bears, the wither and skeleton horse traps follow vanilla | Ported here from each mob's class and goals |
| Armadillos run on vanilla's brain (rolling up, scutes, brushing) | Ported here from `Armadillo` and `ArmadilloAi` |
| Frogs, tadpoles, allays, breezes, sniffers, goats, hoglins, zoglins, creakings, nautiluses and zombie nautiluses run on vanilla's brains | Ported here from each mob's `*Ai` |
| The ender dragon and its phases, and dragon fireballs | Ported here from `EnderDragon` |
| Blazes hover and shoot like vanilla; small and large fireballs, wither skulls and dragon fireballs fly like `AbstractHurtingProjectile`; experience orbs are vanilla's `ExperienceOrb` | Ported here |
| Spiders and cave spiders climb walls with `WallClimberNavigation`; cave spiders skip `Spider`'s spawn setup | Ported here |
| Collisions resolve one axis at a time like `collideWithShapes`, and walking mobs step up anything within their step height | Ported here from `Entity.collide` |
| Passengers tick with their vehicle and sit where vanilla puts them, from vanilla's attachment points; saddles are equipment, and mount inventories, riding jumps and steered vehicles follow vanilla; `/summon` loads `Passengers` | Ported here from `positionRider`, `EntityAttachments` and `AbstractHorse` |
| Primed TNT falls like `PrimedTnt`; a dying mob stops on the tick it is removed | Fixed here |
| Post-attack enchantment effects (Thorns and others), armour wear, the mace smash, data-driven Unbreaking, Soul Speed and Frost Walker, difficulty damage scaling, vanilla fall distance | Ported here from `EnchantmentHelper` and `LivingEntity` |
| `execute if`/`unless` and `execute store`, functions with `return`, macros and limits, `/team`, `/swing`, `/compute` and `/data` | Ported here from vanilla's commands |
| Saves use 26.3's data version and block state codec, keep chunk tags Pumpkin doesn't model, and keep game time, world clocks and the world border; scheduled ticks take any delay | Ported here |
| Clients get vanilla's recipe sync (furnace, smithing and brewing inputs, stonecutter recipes), so the stonecutter works and JEI starts on join | Ported here from `ClientboundUpdateRecipesPacket` |
| Anvil stays the default chunk format; upstream's `.pump` format and `--convert-world` are available | Kept here over upstream's default |
| `/effect` targets any entity | Fixed here |

## Issues

Open issues from upstream are mirrored into this repository, with `[upstream N]` at the end
of the title and a link back to the original. An issue is closed here when it is fixed in
this fork, which says nothing about its state upstream. Please report upstream bugs upstream.

## Running a debug build

Debug builds overflow the stack during chunk generation unless threads get a larger stack
(release builds manage with 32 MiB):

```bash
RUST_MIN_STACK=268435456 cargo run -p pumpkin
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
