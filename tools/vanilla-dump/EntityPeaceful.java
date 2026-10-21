// Dumps EntityType.isAllowedInPeaceful (the builder's notInPeaceful) of every entity type from a
// real vanilla server.
// Run from testing/vanilla-test (needs its versions/ and libraries/):
//   CP=$(find versions libraries -name '*.jar' | tr '\n' ':')
//   java -cp "$CP" EntityPeaceful.java out.json
import java.io.FileWriter;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.EntityType;

public class EntityPeaceful {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Map<String, Boolean> types = new TreeMap<>();
        for (EntityType<?> type : BuiltInRegistries.ENTITY_TYPE) {
            types.put(BuiltInRegistries.ENTITY_TYPE.getKey(type).getPath(), type.isAllowedInPeaceful());
        }
        List<String> entries = new ArrayList<>();
        for (Map.Entry<String, Boolean> entry : types.entrySet()) {
            entries.add("  \"" + entry.getKey() + "\": " + entry.getValue());
        }
        try (FileWriter w = new FileWriter(args[0])) {
            w.write("{\n" + String.join(",\n", entries) + "\n}\n");
        }
    }
}
