package dev.sunriselab.parity;

import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.screens.inventory.tooltip.TooltipRenderUtil;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.component.DataComponents;
import net.minecraft.resources.Identifier;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.TooltipFlag;
import net.minecraft.world.item.component.TooltipDisplay;

/** The name, attribute lines, and tooltip_style sprites returned for item tooltips. */
public final class VanillaTooltipDisplayOracle {
    /** Records blitSprite calls made by TooltipRenderUtil.extractTooltipBackground. */
    public static final class TooltipCapture extends GuiGraphicsExtractor {
        List<String> draws;

        private TooltipCapture() {
            super(null, null, 0, 0);
        }

        @Override
        public void blitSprite(RenderPipeline pipeline, Identifier location, int x, int y, int width, int height, int color) {
            draws.add(location + "," + x + "," + y + "," + width + "," + height + "," + Integer.toUnsignedString(color));
        }
    }

    private static TooltipCapture capture() throws Exception {
        Field unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        Object unsafe = unsafeField.get(null);
        TooltipCapture capture = (TooltipCapture) unsafe.getClass()
            .getMethod("allocateInstance", Class.class)
            .invoke(unsafe, TooltipCapture.class);
        capture.draws = new ArrayList<>();
        return capture;
    }

    public static void main(String[] args) throws Exception {
        ParityData.init();
        var context = Item.TooltipContext.of(ParityData.lookup);
        var output = new ArrayList<String>();
        for (String line : Files.readAllLines(Path.of(args[0]))) {
            if (line.startsWith("name\t")) {
                var stack = ParityData.stack("{id:\"minecraft:stick\",count:1,components:{\"minecraft:custom_name\":" + line.substring(5) + "}}");
                var result = new StringBuilder("name\t");
                stack.getStyledHoverName().visit((style, text) -> {
                    text.codePoints().forEach(c -> result.append(c).append(',')
                        .append(style.getColor() == null ? 0xffffff : style.getColor().getValue()).append(',')
                        .append(style.isBold()).append(',').append(style.isItalic()).append(',')
                        .append(style.isUnderlined()).append(',').append(style.isStrikethrough()).append(',')
                        .append(style.isObfuscated()).append('|'));
                    return java.util.Optional.empty();
                }, net.minecraft.network.chat.Style.EMPTY);
                output.add(result.toString());
                continue;
            }
            if (line.startsWith("style\t")) {
                String[] fields = line.split("\t", 6);
                int x = Integer.parseInt(fields[1]);
                int y = Integer.parseInt(fields[2]);
                int w = Integer.parseInt(fields[3]);
                int h = Integer.parseInt(fields[4]);
                var stack = ParityData.stack(fields[5]);
                var capture = capture();
                TooltipRenderUtil.extractTooltipBackground(capture, x, y, w, h, stack.get(DataComponents.TOOLTIP_STYLE));
                output.add("style\t" + String.join("|", capture.draws));
                continue;
            }
            String[] fields = line.split("\t");
            boolean hideTooltip = Boolean.parseBoolean(fields[0]);
            boolean creative = Boolean.parseBoolean(fields[1]);
            boolean hideAttributes = Boolean.parseBoolean(fields[2]);
            var hidden = new LinkedHashSet<DataComponentType<?>>();
            if (hideAttributes) hidden.add(DataComponents.ATTRIBUTE_MODIFIERS);
            var stack = new ItemStack(Items.DIAMOND_SWORD, 1);
            stack.set(DataComponents.TOOLTIP_DISPLAY, new TooltipDisplay(hideTooltip, hidden));
            TooltipFlag flag = creative ? TooltipFlag.Default.NORMAL.asCreative() : TooltipFlag.Default.NORMAL;
            var tooltip = stack.getTooltipLines(context, null, flag);
            output.add((!tooltip.isEmpty()) + "\t" + (tooltip.size() > 1));
        }
        Files.write(Path.of(args[1]), output);
    }
}
