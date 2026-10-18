// Dumps the PASSENGER and VEHICLE attachment points of every entity type's dimensions and of
// every static EntityDimensions in the entity classes (baby sizes, poses) from a real vanilla
// server, as the doubles vanilla holds after EntityAttachments.Builder.build and any scaling.
// Run from testing/vanilla-test (needs its versions/ and libraries/):
//   CP=$(find versions libraries -name '*.jar' | tr '\n' ':')
//   java -cp "$CP" EntityAttachments.java out.json
import java.io.FileWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Enumeration;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.jar.JarEntry;
import java.util.jar.JarFile;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.EntityAttachment;
import net.minecraft.world.entity.EntityDimensions;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.phys.Vec3;

public class EntityAttachments {
    static Field POINTS;

    static String points(List<Vec3> list) {
        List<String> out = new ArrayList<>();
        for (Vec3 v : list) {
            out.add("[" + v.x + ", " + v.y + ", " + v.z + "]");
        }
        return "[" + String.join(", ", out) + "]";
    }

    @SuppressWarnings("unchecked")
    static String dims(EntityDimensions d) throws Exception {
        Map<EntityAttachment, List<Vec3>> map =
            (Map<EntityAttachment, List<Vec3>>) POINTS.get(d.attachments());
        return String.format("{\"width\": %s, \"height\": %s, \"eye_height\": %s, \"passenger\": %s, \"vehicle\": %s}",
            d.width(), d.height(), d.eyeHeight(),
            points(map.get(EntityAttachment.PASSENGER)), points(map.get(EntityAttachment.VEHICLE)));
    }

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        POINTS = net.minecraft.world.entity.EntityAttachments.class.getDeclaredField("attachments");
        POINTS.setAccessible(true);

        Map<String, String> types = new TreeMap<>();
        for (EntityType<?> type : BuiltInRegistries.ENTITY_TYPE) {
            types.put(BuiltInRegistries.ENTITY_TYPE.getKey(type).getPath(), dims(type.getDimensions()));
        }

        Map<String, String> statics = new TreeMap<>();
        String jar = EntityType.class.getProtectionDomain().getCodeSource().getLocation().getPath();
        try (JarFile file = new JarFile(jar)) {
            Enumeration<JarEntry> entries = file.entries();
            while (entries.hasMoreElements()) {
                String name = entries.nextElement().getName();
                if (!name.startsWith("net/minecraft/world/entity/") || !name.endsWith(".class")) continue;
                Class<?> cls;
                try {
                    cls = Class.forName(name.replace('/', '.').replace(".class", ""));
                } catch (Throwable e) {
                    continue;
                }
                for (Field f : cls.getDeclaredFields()) {
                    if (!Modifier.isStatic(f.getModifiers())) continue;
                    String key = cls.getName().replace("net.minecraft.world.entity.", "") + "." + f.getName();
                    try {
                        f.setAccessible(true);
                        if (f.getType() == EntityDimensions.class) {
                            statics.put(key, dims((EntityDimensions) f.get(null)));
                        } else if (Map.class.isAssignableFrom(f.getType()) && f.get(null) instanceof Map<?, ?> m) {
                            for (Map.Entry<?, ?> e : m.entrySet()) {
                                if (e.getValue() instanceof EntityDimensions d) {
                                    statics.put(key + "." + e.getKey(), dims(d));
                                }
                            }
                        }
                    } catch (Throwable e) {
                        // Inaccessible or not initialisable: not a dimension holder.
                    }
                }
            }
        }

        StringBuilder out = new StringBuilder("{\n  \"types\": {\n");
        out.append(String.join(",\n", types.entrySet().stream()
            .map(e -> "    \"" + e.getKey() + "\": " + e.getValue()).toList()));
        out.append("\n  },\n  \"statics\": {\n");
        out.append(String.join(",\n", statics.entrySet().stream()
            .map(e -> "    \"" + e.getKey() + "\": " + e.getValue()).toList()));
        out.append("\n  }\n}\n");
        try (FileWriter w = new FileWriter(args[0])) {
            w.write(out.toString());
        }
    }
}
