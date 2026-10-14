// Dumps every block state's SoundType from a real vanilla server.
// Run from testing/vanilla-test (needs its versions/ and libraries/):
//   CP=$(find versions libraries -name '*.jar' | tr '\n' ':')
//   java -cp "$CP" ../scripts/dump/BlockSoundTypes.java out.json
import java.io.FileWriter;
import java.lang.reflect.Field;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.IdentityHashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockState;

public class BlockSoundTypes {
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        Map<SoundType, String> names = new IdentityHashMap<>();
        StringBuilder types = new StringBuilder();
        for (Field f : SoundType.class.getDeclaredFields()) {
            int m = f.getModifiers();
            if (!Modifier.isStatic(m) || f.getType() != SoundType.class) continue;
            SoundType t = (SoundType) f.get(null);
            names.put(t, f.getName());
            if (types.length() > 0) types.append(",\n");
            types.append(String.format(
                "    \"%s\": {\"volume\": %s, \"pitch\": %s, \"break\": \"%s\", \"step\": \"%s\", \"place\": \"%s\", \"hit\": \"%s\", \"fall\": \"%s\"}",
                f.getName(), t.getVolume(), t.getPitch(),
                t.getBreakSound().location().getPath(), t.getStepSound().location().getPath(),
                t.getPlaceSound().location().getPath(), t.getHitSound().location().getPath(),
                t.getFallSound().location().getPath()));
        }

        StringBuilder blocks = new StringBuilder();
        for (Block block : BuiltInRegistries.BLOCK) {
            List<String> perState = new ArrayList<>();
            for (BlockState state : block.getStateDefinition().getPossibleStates()) {
                String n = names.get(state.getSoundType());
                if (n == null) throw new IllegalStateException("unnamed SoundType on " + state);
                perState.add(n);
            }
            boolean same = perState.stream().allMatch(perState.get(0)::equals);
            if (blocks.length() > 0) blocks.append(",\n");
            blocks.append("    \"").append(BuiltInRegistries.BLOCK.getKey(block).getPath()).append("\": ");
            if (same) {
                blocks.append('"').append(perState.get(0)).append('"');
            } else {
                // One entry per state, in the block's state order (same order as blocks.json).
                blocks.append('[');
                for (int i = 0; i < perState.size(); i++) {
                    if (i > 0) blocks.append(", ");
                    blocks.append('"').append(perState.get(i)).append('"');
                }
                blocks.append(']');
            }
        }

        try (FileWriter w = new FileWriter(args[0])) {
            w.write("{\n  \"sound_types\": {\n" + types + "\n  },\n  \"blocks\": {\n" + blocks + "\n  }\n}\n");
        }
    }
}
