// Dumps the default value of every synced entity data item (Entity's constructor defines plus
// defineSynchedData) of every entity type from a real vanilla server, as the exact bytes
// SynchedEntityData.DataValue.write sends: index byte, serializer varint, value. The entity is
// allocated without running its constructor and given a stub level whose registry access holds
// the vanilla datapack's world registries, so holder-typed defaults (variants) resolve.
// Run from testing/vanilla-test (needs its versions/ and libraries/):
//   CP=$(find versions libraries -name '*.jar' | tr '\n' ':')
//   java -cp "$CP" SynchedDataDefaults.java out.json
// Failures (per type or per item) are listed on stderr; the output is the remaining items.
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.io.FileWriter;
import java.lang.reflect.Field;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.lang.reflect.ParameterizedType;
import java.lang.reflect.Type;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.TreeMap;
import net.minecraft.SharedConstants;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.LayeredRegistryAccess;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.network.syncher.EntityDataAccessor;
import net.minecraft.network.syncher.EntityDataSerializers;
import net.minecraft.network.syncher.SynchedEntityData;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.util.RandomSource;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.EntityTypes;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.level.Level;
import sun.misc.Unsafe;

public class SynchedDataDefaults {
    static Unsafe UNSAFE;

    static Field field(Class<?> cls, String name) throws Exception {
        Field f = cls.getDeclaredField(name);
        f.setAccessible(true);
        return f;
    }

    @SuppressWarnings("unchecked")
    static <T> EntityDataAccessor<T> accessor(String name) throws Exception {
        return (EntityDataAccessor<T>) field(Entity.class, name).get(null);
    }

    static RegistryAccess.Frozen registries() throws Exception {
        // The packs a default server selects: just "vanilla".
        PackRepository packs = ServerPacksSource.createVanillaTrustedRepository();
        packs.reload();
        packs.setSelected(List.of("vanilla"));
        MultiPackResourceManager resources = new MultiPackResourceManager(PackType.SERVER_DATA, packs.openAllSelected());
        LayeredRegistryAccess<RegistryLayer> layers = RegistryLayer.createRegistryAccess();
        List<Registry.PendingTags<?>> staticTags =
            TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
        List<HolderLookup.RegistryLookup<?>> context =
            TagLoader.buildUpdatedLookups(layers.getAccessForLoading(RegistryLayer.WORLD), staticTags);
        RegistryAccess.Frozen world =
            RegistryDataLoader.load(resources, context, RegistryDataLoader.WORLD_REGISTRIES, Runnable::run).get();
        return layers.replaceFrom(RegistryLayer.WORLD, world).compositeAccess();
    }

    /** The entity class each EntityTypes constant is declared with (`EntityType<Cow> COW`). */
    static Map<EntityType<?>, Class<?>> entityClasses() throws Exception {
        Map<EntityType<?>, Class<?>> out = new HashMap<>();
        for (Field f : EntityTypes.class.getDeclaredFields()) {
            if (!Modifier.isStatic(f.getModifiers()) || f.getType() != EntityType.class) continue;
            Type generic = f.getGenericType();
            if (!(generic instanceof ParameterizedType p)) continue;
            Type arg = p.getActualTypeArguments()[0];
            if (arg instanceof ParameterizedType pa) arg = pa.getRawType();
            if (arg instanceof Class<?> c) out.put((EntityType<?>) f.get(null), c);
        }
        return out;
    }

    static String hex(ByteBuf buf) {
        StringBuilder sb = new StringBuilder();
        while (buf.isReadable()) sb.append(String.format("%02x", buf.readByte() & 0xff));
        return sb.toString();
    }

    static final int SEEDS = 16;

    /** Builds the entity's default SynchedEntityData like Entity's constructor and encodes each item. */
    static Map<Integer, String> encode(EntityType<?> type, Class<?> cls, Level level, RegistryAccess access,
                                       long seed, String name, List<String> failures) throws Throwable {
        Entity entity = (Entity) UNSAFE.allocateInstance(cls);
        field(Entity.class, "type").set(entity, type);
        field(Entity.class, "level").set(entity, level);
        field(Entity.class, "random").set(entity, RandomSource.create(seed));
        // Mirrors Entity's constructor.
        SynchedEntityData.Builder builder = new SynchedEntityData.Builder(entity);
        builder.define(accessor("DATA_SHARED_FLAGS_ID"), (byte) 0);
        builder.define(accessor("DATA_AIR_SUPPLY_ID"), entity.getMaxAirSupply());
        builder.define(accessor("DATA_CUSTOM_NAME_VISIBLE"), false);
        builder.define(accessor("DATA_CUSTOM_NAME"), Optional.empty());
        builder.define(accessor("DATA_SILENT"), false);
        builder.define(accessor("DATA_NO_GRAVITY"), false);
        builder.define(accessor("DATA_POSE"), Pose.STANDING);
        builder.define(accessor("DATA_TICKS_FROZEN"), 0);
        Method define = Entity.class.getDeclaredMethod("defineSynchedData", SynchedEntityData.Builder.class);
        define.setAccessible(true);
        define.invoke(entity, builder);
        SynchedEntityData.DataItem<?>[] built =
            (SynchedEntityData.DataItem<?>[]) field(SynchedEntityData.class, "itemsById").get(builder.build());

        Map<Integer, String> values = new TreeMap<>();
        for (SynchedEntityData.DataItem<?> item : built) {
            int index = item.getAccessor().id();
            try {
                RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), access);
                item.value().write(buf);
                values.put(index, hex(buf));
            } catch (Throwable e) {
                failures.add(name + " #" + index + " (serializer "
                    + EntityDataSerializers.getSerializedId(item.getAccessor().serializer()) + "): " + e);
            }
        }
        return values;
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        UNSAFE = (Unsafe) field(Unsafe.class, "theUnsafe").get(null);

        RegistryAccess.Frozen access = registries();
        // ReloadableServerResources binds item components; default ItemStacks need them.
        BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(access).forEach(p -> p.apply());
        ServerLevel level = (ServerLevel) UNSAFE.allocateInstance(ServerLevel.class);
        field(Level.class, "registryAccess").set(level, access);

        Map<EntityType<?>, Class<?>> classes = entityClasses();
        Map<String, Map<Integer, String>> out = new TreeMap<>();
        List<String> failures = new ArrayList<>();
        int items = 0;
        for (EntityType<?> type : BuiltInRegistries.ENTITY_TYPE) {
            String name = BuiltInRegistries.ENTITY_TYPE.getKey(type).getPath();
            Class<?> cls = classes.get(type);
            // Player is abstract; the server's players are ServerPlayer.
            if (cls == Player.class) cls = ServerPlayer.class;
            if (cls == null) {
                failures.add(name + ": no entity class");
                continue;
            }
            Map<Integer, String> values;
            try {
                values = encode(type, cls, level, access, 0, name, failures);
                // A default drawn from the entity's random (ZombieVillager's profession) has no
                // fixed value the client could assume: drop items that differ between seeds.
                for (long seed = 1; seed < SEEDS; seed++) {
                    Map<Integer, String> other = encode(type, cls, level, access, seed, name, new ArrayList<>());
                    for (var it = values.entrySet().iterator(); it.hasNext(); ) {
                        var e = it.next();
                        if (!e.getValue().equals(other.get(e.getKey()))) {
                            failures.add(name + " #" + e.getKey() + ": random default");
                            it.remove();
                        }
                    }
                }
            } catch (Throwable e) {
                Throwable cause = e instanceof InvocationTargetException ite ? ite.getCause() : e;
                failures.add(name + " (" + cls.getSimpleName() + "): " + cause);
                continue;
            }
            items += values.size();
            out.put(name, values);
        }

        StringBuilder json = new StringBuilder("{\n");
        json.append(String.join(",\n", out.entrySet().stream()
            .map(t -> "  \"" + t.getKey() + "\": {"
                + String.join(", ", t.getValue().entrySet().stream()
                    .map(v -> "\"" + v.getKey() + "\": \"" + v.getValue() + "\"").toList())
                + "}")
            .toList()));
        json.append("\n}\n");
        try (FileWriter w = new FileWriter(args[0])) {
            w.write(json.toString());
        }
        System.err.println(out.size() + " entity types, " + items + " items, " + failures.size() + " failures");
        for (String f : failures) System.err.println("  " + f);
    }
}
