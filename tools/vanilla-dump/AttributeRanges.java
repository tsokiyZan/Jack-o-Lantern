// Dumps the RangedAttribute min and max of every attribute from a real vanilla server, for
// AttributeInstance's sanitizeValue, and whether the attribute is client syncable
// (Attribute.isClientSyncable, for the attribute sync).
// Run from testing/vanilla-test (needs its versions/ and libraries/):
//   CP=$(find versions libraries -name '*.jar' | tr '\n' ':')
//   java -cp "$CP" AttributeRanges.java out.json
import java.io.FileWriter;
import java.util.Map;
import java.util.TreeMap;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.ai.attributes.Attribute;
import net.minecraft.world.entity.ai.attributes.RangedAttribute;

public class AttributeRanges {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Map<String, String> ranges = new TreeMap<>();
        for (Attribute attribute : BuiltInRegistries.ATTRIBUTE) {
            if (attribute instanceof RangedAttribute ranged) {
                ranges.put(BuiltInRegistries.ATTRIBUTE.getKey(attribute).getPath(),
                    "{\"min_value\": " + ranged.getMinValue() + ", \"max_value\": " + ranged.getMaxValue()
                    + ", \"syncable\": " + ranged.isClientSyncable() + "}");
            }
        }
        StringBuilder out = new StringBuilder("{\n");
        out.append(String.join(",\n", ranges.entrySet().stream()
            .map(e -> "  \"" + e.getKey() + "\": " + e.getValue()).toList()));
        out.append("\n}\n");
        try (FileWriter w = new FileWriter(args[0])) {
            w.write(out.toString());
        }
    }
}
