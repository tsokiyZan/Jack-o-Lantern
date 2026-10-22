use super::BlockEntity;
use crate::entity::living::LivingEntity;
use crate::world::World;
use crate::world::loot::{LootContextParameters, container_unpack_params};
use pumpkin_data::attributes::Attributes;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_nbt::compound::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use std::sync::Mutex;

pub struct BrushableBlockBlockEntity {
    pub position: BlockPos,
    pub item: Mutex<Option<ItemStack>>,
    pub hits: Mutex<i32>,
    pub hit_direction: Mutex<Option<u8>>,
    pub loot_table: Mutex<Option<String>>,
    pub loot_table_seed: Mutex<i64>,
}

impl BlockEntity for BrushableBlockBlockEntity {
    fn resource_location(&self) -> &'static str {
        Self::ID
    }

    fn get_position(&self) -> BlockPos {
        self.position
    }

    fn from_nbt(nbt: &pumpkin_nbt::compound::NbtCompound, position: BlockPos) -> Self
    where
        Self: Sized,
    {
        let item = nbt
            .get_compound("item")
            .and_then(ItemStack::read_item_stack);
        let hits = nbt.get_int("hits").unwrap_or(0);
        let hit_direction = nbt
            .get_byte("hit_direction")
            .or_else(|| nbt.get_byte("direction"))
            .map(|b| b as u8);
        let loot_table = nbt.get_string("LootTable").map(ToString::to_string);
        let loot_table_seed = nbt.get_long("LootTableSeed").unwrap_or(0);
        Self {
            position,
            item: Mutex::new(item),
            hits: Mutex::new(hits),
            hit_direction: Mutex::new(hit_direction),
            loot_table: Mutex::new(loot_table),
            loot_table_seed: Mutex::new(loot_table_seed),
        }
    }

    fn write_nbt(&self, nbt: &mut NbtCompound) {
        if let Ok(loot_table) = self.loot_table.lock()
            && let Some(table) = loot_table.as_ref()
        {
            nbt.put_string("LootTable", table.clone());
            if let Ok(seed) = self.loot_table_seed.lock()
                && *seed != 0
            {
                nbt.put_long("LootTableSeed", *seed);
            }
        } else if let Ok(item) = self.item.lock()
            && let Some(it) = item.as_ref()
        {
            let mut it_nbt = NbtCompound::new();
            it.write_item_stack(&mut it_nbt);
            nbt.put_compound("item", it_nbt);
        }
        if let Ok(hits) = self.hits.lock()
            && *hits != 0
        {
            nbt.put_int("hits", *hits);
        }
        if let Ok(direction) = self.hit_direction.lock()
            && let Some(dir) = *direction
        {
            nbt.put_byte("hit_direction", dir as i8);
        }
    }

    fn chunk_data_nbt(&self) -> Option<NbtCompound> {
        let mut nbt = NbtCompound::new();
        if let Ok(item) = self.item.try_lock()
            && let Some(ref it) = *item
        {
            let mut it_nbt = NbtCompound::new();
            it.write_item_stack(&mut it_nbt);
            nbt.put_compound("item", it_nbt);
        }
        if let Ok(direction) = self.hit_direction.try_lock()
            && let Some(dir) = *direction
        {
            nbt.put_byte("hit_direction", dir as i8);
        }
        Some(nbt)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl BrushableBlockBlockEntity {
    pub const ID: &'static str = "minecraft:brushable_block";

    /// Vanilla `BrushableBlockEntity.unpackLootTable`: the stored loot table, rolled once with
    /// its seed, becomes the item (none when it rolls nothing).
    pub fn unpack_loot_table(&self, world: &World, user: &LivingEntity, brush: &ItemStack) {
        let Some(key) = self
            .loot_table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let seed = *self
            .loot_table_seed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // A missing table is vanilla's `LootTable.EMPTY`.
        let loot = world.get_loot_table(&key).map_or_else(Vec::new, |table| {
            let params = LootContextParameters {
                luck: user.get_attribute_value(&Attributes::LUCK) as f32,
                this_entity: Some(user.entity.entity_type),
                tool: Some(brush.clone()),
                ..container_unpack_params(self.position.to_centered_f64(), None)
            };
            crate::world::loot::generate_loot_from_handle(&table, world, seed, &params)
        });
        if loot.len() > 1 {
            tracing::warn!(
                "Expected max 1 loot from loot table {key}, but got {}",
                loot.len()
            );
        }
        *self
            .item
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = loot.into_iter().next();
    }
    #[must_use]
    pub const fn new(position: BlockPos) -> Self {
        Self {
            position,
            item: Mutex::new(None),
            hits: Mutex::new(0),
            hit_direction: Mutex::new(None),
            loot_table: Mutex::new(None),
            loot_table_seed: Mutex::new(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::test_support::test_world;
    use pumpkin_data::entity::EntityType;
    use pumpkin_data::item::Item;
    use pumpkin_util::math::vector3::Vector3;

    fn unpacked(loot_table: &str, seed: i64) -> Option<ItemStack> {
        let (_folder, world) = test_world();
        let mut nbt = NbtCompound::new();
        nbt.put_string("LootTable", loot_table.to_string());
        nbt.put_long("LootTableSeed", seed);
        let entity = BrushableBlockBlockEntity::from_nbt(&nbt, BlockPos::new(0, 0, 0));
        let user = crate::entity::r#type::from_type(
            &EntityType::ZOMBIE,
            Vector3::new(0.0, 100.0, 0.0),
            &world,
            uuid::Uuid::new_v4(),
        );
        entity.unpack_loot_table(
            &world,
            user.get_living_entity().unwrap(),
            &ItemStack::new(1, &Item::BRUSH),
        );
        assert!(entity.loot_table.lock().unwrap().is_none());
        entity.item.lock().unwrap().take()
    }

    /// The seed is a `LegacyRandomSource`: `desert_pyramid`'s one pool picks `nextInt(8)` of
    /// its eight entries (java.util.Random(42) gives 5, Random(12345) gives 2).
    #[tokio::test]
    async fn unpack_rolls_the_stored_table_with_its_seed() {
        let table = "minecraft:archaeology/desert_pyramid";
        assert_eq!(unpacked(table, 42).unwrap().item.id, Item::TNT.id);
        assert_eq!(
            unpacked(table, 12345).unwrap().item.id,
            Item::PRIZE_POTTERY_SHERD.id
        );
    }

    /// A table that doesn't exist is `LootTable.EMPTY`: nothing to drop.
    #[tokio::test]
    async fn unpack_of_a_missing_table_is_empty() {
        assert!(unpacked("minecraft:archaeology/missing", 1).is_none());
    }
}
