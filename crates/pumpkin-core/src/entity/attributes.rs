use pumpkin_data::attributes::Attributes;
use pumpkin_data::entity::EntityType;
use rustc_hash::FxHashMap;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

#[derive(Clone, Debug, Copy, PartialEq, Eq)]
#[repr(i8)]
pub enum ModifierOperation {
    Add = 0,           // add value
    MultiplyBase = 1,  // multiply base (base * (1 + x))
    MultiplyTotal = 2, // multiply total (applied last)
}

impl ModifierOperation {
    /// The item component and enchantment `Operation` as the runtime modifier operation.
    #[must_use]
    pub const fn from_operation(operation: pumpkin_data::data_component_impl::Operation) -> Self {
        use pumpkin_data::data_component_impl::Operation;
        match operation {
            Operation::AddValue => Self::Add,
            Operation::AddMultipliedBase => Self::MultiplyBase,
            Operation::AddMultipliedTotal => Self::MultiplyTotal,
        }
    }

    /// Vanilla `AttributeModifier.Operation.getSerializedName`.
    #[must_use]
    pub const fn serialized_name(self) -> &'static str {
        match self {
            Self::Add => "add_value",
            Self::MultiplyBase => "add_multiplied_base",
            Self::MultiplyTotal => "add_multiplied_total",
        }
    }

    #[must_use]
    pub fn from_serialized_name(name: &str) -> Option<Self> {
        match name {
            "add_value" => Some(Self::Add),
            "add_multiplied_base" => Some(Self::MultiplyBase),
            "add_multiplied_total" => Some(Self::MultiplyTotal),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    pub id: String,
    pub amount: f64,
    pub operation: ModifierOperation,
}

/// Per-entity attribute instance used at runtime.
#[derive(Debug)]
pub struct AttributeInstance {
    pub base_value: f64,
    pub modifiers: Vec<Modifier>,
    /// Ids of the modifiers vanilla saves with the entity (`permanentModifiers`).
    pub permanent_modifiers: Vec<String>,
    pub cached_value: AtomicU64,
    pub dirty: AtomicBool,
}

impl AttributeInstance {
    #[must_use]
    pub const fn new(base_value: f64) -> Self {
        Self {
            base_value,
            modifiers: Vec::new(),
            permanent_modifiers: Vec::new(),
            cached_value: AtomicU64::new(base_value.to_bits()),
            dirty: AtomicBool::new(false),
        }
    }

    pub fn value(&self) -> f64 {
        if !self.dirty.load(Ordering::Relaxed) {
            return f64::from_bits(self.cached_value.load(Ordering::Relaxed));
        }

        // Vanilla `AttributeInstance.calculateValue`, operation by operation in insertion order.
        let modifiers = |operation| {
            self.modifiers
                .iter()
                .filter(move |m| m.operation == operation)
                .map(|m| m.amount)
        };
        let mut base = self.base_value;
        for amount in modifiers(ModifierOperation::Add) {
            base += amount;
        }
        let mut value = base;
        for amount in modifiers(ModifierOperation::MultiplyBase) {
            value += base * amount;
        }
        for amount in modifiers(ModifierOperation::MultiplyTotal) {
            value *= 1.0 + amount;
        }

        if value.is_nan() || value.is_infinite() {
            value = self.base_value;
        }

        self.cached_value.store(value.to_bits(), Ordering::Relaxed);
        self.dirty.store(false, Ordering::Relaxed);

        value
    }

    /// Vanilla `addOrUpdateTransientModifier`: a modifier with the same id is replaced in
    /// place, keeping its position in the order.
    pub fn add_or_replace_modifier(&mut self, modifier: Modifier) {
        if let Some(existing) = self.modifiers.iter_mut().find(|m| m.id == modifier.id) {
            *existing = modifier;
        } else {
            self.modifiers.push(modifier);
        }
        self.dirty.store(true, Ordering::Relaxed);
    }

    pub fn remove_modifier(&mut self, id: &str) {
        if let Some(pos) = self.modifiers.iter().position(|m| m.id == id) {
            self.modifiers.remove(pos);
        }
        self.permanent_modifiers.retain(|permanent| permanent != id);
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Vanilla `addOrReplacePermanentModifier`: replaces any modifier with the same id, at the
    /// end of the order, and keeps it in the entity's save.
    pub fn add_or_replace_permanent_modifier(&mut self, modifier: Modifier) {
        self.remove_modifier(&modifier.id);
        self.permanent_modifiers.push(modifier.id.clone());
        self.modifiers.push(modifier);
        self.dirty.store(true, Ordering::Relaxed);
    }

    #[must_use]
    pub fn get_modifier(&self, id: &str) -> Option<&Modifier> {
        self.modifiers.iter().find(|m| m.id == id)
    }

    /// The modifiers vanilla's `AttributeInstance.pack` saves.
    pub fn permanent_modifiers(&self) -> impl Iterator<Item = &Modifier> {
        self.modifiers
            .iter()
            .filter(|m| self.permanent_modifiers.contains(&m.id))
    }
}

/// Send updates for multiple attributes in a single packet for the given living entity.
pub fn send_attribute_updates_for_living(
    living: &crate::entity::living::LivingEntity,
    attributes: Vec<Attributes>,
) {
    use pumpkin_protocol::bedrock::client::update_attributes::{
        AttributeData as BeAttribute, CUpdateAttributes as BePacket,
    };
    use pumpkin_protocol::codec::var_int::VarInt;
    use pumpkin_protocol::codec::var_ulong::VarULong;
    use pumpkin_protocol::java::client::play::AttributeModifier as JeAttrMod;
    use pumpkin_protocol::java::client::play::CUpdateAttributes as JePacket;
    use pumpkin_protocol::java::client::play::Property as JeProperty;

    let mut je_properties: Vec<JeProperty> = Vec::with_capacity(attributes.len());
    let mut be_attributes: Vec<BeAttribute> = Vec::with_capacity(attributes.len());

    for attribute in attributes {
        let base_value = living.get_attribute_base(&attribute);
        let effective_value = living.get_attribute_value(&attribute);

        // Pull modifiers for this attribute
        let mut modifiers = Vec::new();
        if let Some(inst) = living
            .attributes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&attribute.id)
        {
            for mod_inst in &inst.modifiers {
                modifiers.push(JeAttrMod::new(
                    mod_inst.id.clone(),
                    mod_inst.amount,
                    mod_inst.operation as i8,
                ));
            }
        }

        // Move modifiers into the property
        je_properties.push(JeProperty::new(
            VarInt(i32::from(attribute.id)),
            base_value,
            modifiers,
        ));

        let name = match attribute.id {
            id if id == Attributes::MOVEMENT_SPEED.id => "minecraft:movement".to_string(),
            id if id == Attributes::MAX_HEALTH.id => "minecraft:health".to_string(),
            id if id == Attributes::MAX_ABSORPTION.id => "minecraft:absorption".to_string(),
            id if id == Attributes::ATTACK_DAMAGE.id => "minecraft:attack_damage".to_string(),
            id if id == Attributes::KNOCKBACK_RESISTANCE.id => {
                "minecraft:knockback_resistance".to_string()
            }
            id if id == Attributes::LUCK.id => "minecraft:luck".to_string(),
            id if id == Attributes::FOLLOW_RANGE.id => "minecraft:follow_range".to_string(),
            id if id == Attributes::JUMP_STRENGTH.id => "minecraft:horse.jump_strength".to_string(),
            // Java-only attributes must not be sent under unsupported Bedrock names.
            _ => continue,
        };

        let be_attribute = BeAttribute {
            min_value: 0.0,
            max_value: 3.402_823_5E38,
            current_value: effective_value as f32,
            default_min_value: 0.0,
            default_max_value: 3.402_823_5E38,
            default_value: base_value as f32,
            name,
            // Bedrock receives the already-computed effective value above. Do not advertise
            // modifier entries until their payload is encoded as well.
            modifiers: Vec::new(),
        };

        be_attributes.push(be_attribute);
    }

    let je_packet = JePacket::new(living.entity.entity_id.into(), je_properties);
    let world = living.entity.world.load();
    if be_attributes.is_empty() {
        world.broadcast_packet_all(&je_packet);
        return;
    }

    let runtime_id = living.entity.entity_id as u64;
    let be_packet = BePacket {
        target_runtime_id: VarULong(runtime_id),
        attribute_list: be_attributes,
        tick: VarULong(0),
    };

    world.broadcast_editioned(&je_packet, &be_packet);
}

impl Clone for AttributeInstance {
    fn clone(&self) -> Self {
        Self {
            base_value: self.base_value,
            modifiers: self.modifiers.clone(),
            permanent_modifiers: self.permanent_modifiers.clone(),
            cached_value: AtomicU64::new(self.cached_value.load(Ordering::Relaxed)),
            dirty: AtomicBool::new(self.dirty.load(Ordering::Relaxed)),
        }
    }
}

/// Registry storing per-entity-type base attribute overrides.
/// Internally stores a map from `entity_type.id` -> `FxHashMap`<attribute.id, f64> for O(1) lookup.
#[derive(Default)]
pub struct AttributeRegistry {
    map: FxHashMap<u16, FxHashMap<u8, f64>>,
}

impl AttributeRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the base value for `attribute` for the given entity type id.
    /// If no override exists, returns `attribute.default_value`.
    #[must_use]
    pub fn get_base_value(&self, entity_type_id: u16, attribute: &Attributes) -> f64 {
        self.map
            .get(&entity_type_id)
            .and_then(|map| map.get(&attribute.id))
            .copied()
            .unwrap_or(attribute.default_value)
    }

    /// Return a vector of overrides for the given entity type id.
    /// This allows populating per-entity local attribute instances at spawn time.
    #[must_use]
    pub fn get_overrides_for_entity(&self, entity_type_id: u16) -> Option<Vec<(u8, f64)>> {
        self.map
            .get(&entity_type_id)
            .map(|m| m.iter().map(|(&k, &v)| (k, v)).collect())
    }
}

/// Builder to declaratively assemble attribute overrides for an entity type.
#[derive(Default)]
pub struct AttributeBuilder {
    entries: Vec<(Attributes, f64)>,
}

impl AttributeBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn add(mut self, attribute: Attributes, base: f64) -> Self {
        self.entries.push((attribute, base));
        self
    }

    #[must_use]
    pub fn build(self) -> Vec<(Attributes, f64)> {
        self.entries
    }
}

impl AttributeRegistry {
    /// Register overrides created by an `AttributeBuilder` for `entity_type`.
    pub fn register_builder(
        &mut self,
        entity_type: &'static EntityType,
        builder: AttributeBuilder,
    ) {
        let inner = self.map.entry(entity_type.id).or_default();
        for (attr, val) in builder.build() {
            inner.insert(attr.id, val);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pumpkin_data::attributes::Attributes;
    use pumpkin_data::entity::EntityType;

    #[test]
    fn player_base_attributes() {
        let speed_attr = EntityType::PLAYER
            .attributes
            .iter()
            .find(|(attr, _)| attr.id == Attributes::MOVEMENT_SPEED.id);
        assert!(speed_attr.is_some());
        let (_, base_speed) = speed_attr.unwrap();
        assert!((base_speed - 0.1).abs() < 1e-4);
    }

    #[test]
    fn sprinting_modifier_calculation() {
        let mut instance = AttributeInstance::new(0.1);
        assert!((instance.value() - 0.1).abs() < f64::EPSILON);

        let sprinting_mod = Modifier {
            id: "minecraft:sprinting".to_string(),
            amount: 0.300_000_011_920_928_96,
            operation: ModifierOperation::MultiplyTotal,
        };

        instance.add_or_replace_modifier(sprinting_mod);
        let sprinting_value = instance.value();
        // 0.1 * (1.0 + 0.30000001192092896) = 0.1300000011920929
        assert!((sprinting_value - 0.130_000_001_192_092_9).abs() < 1e-9);

        instance.remove_modifier("minecraft:sprinting");
        assert!((instance.value() - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn attribute_modifier_operations() {
        let mut instance = AttributeInstance::new(10.0);

        // ADD: 10.0 + 2.0 + 3.0 = 15.0
        instance.add_or_replace_modifier(Modifier {
            id: "add_1".to_string(),
            amount: 2.0,
            operation: ModifierOperation::Add,
        });
        instance.add_or_replace_modifier(Modifier {
            id: "add_2".to_string(),
            amount: 3.0,
            operation: ModifierOperation::Add,
        });
        assert!((instance.value() - 15.0).abs() < f64::EPSILON);

        // MULTIPLY_BASE: 15.0 * (1.0 + 0.5 + 0.2) = 15.0 * 1.7 = 25.5
        instance.add_or_replace_modifier(Modifier {
            id: "mul_base_1".to_string(),
            amount: 0.5,
            operation: ModifierOperation::MultiplyBase,
        });
        instance.add_or_replace_modifier(Modifier {
            id: "mul_base_2".to_string(),
            amount: 0.2,
            operation: ModifierOperation::MultiplyBase,
        });
        assert!((instance.value() - 25.5).abs() < f64::EPSILON);

        // MULTIPLY_TOTAL: 25.5 * (1.0 + 0.1) * (1.0 + 0.2) = 25.5 * 1.1 * 1.2 = 33.66
        instance.add_or_replace_modifier(Modifier {
            id: "mul_total_1".to_string(),
            amount: 0.1,
            operation: ModifierOperation::MultiplyTotal,
        });
        instance.add_or_replace_modifier(Modifier {
            id: "mul_total_2".to_string(),
            amount: 0.2,
            operation: ModifierOperation::MultiplyTotal,
        });
        assert!((instance.value() - 33.66).abs() < 1e-9);
    }
}
