package dev.sunriselab.parity;

import java.io.PrintWriter;
import java.nio.file.*;
import net.minecraft.util.StringUtil;

/**
 * Runs the real StringUtil checks the anvil uses (AnvilMenu.setItemName and createResult): each name in the file
 * given (UTF-16 units in hex, "-" for the empty name) through StringUtil.filterText, then StringUtil.isBlank of the
 * result; then every UTF-16 unit with StringUtil.isWhitespace, isAllowedChatCharacter and isBlank of the unit alone
 * ("-" for a lone surrogate, which the Rust side can't hold).
 */
public final class VanillaAnvilNameOracle {
    static String hex(String text) {
        if (text.isEmpty()) return "-";
        var out = new StringBuilder();
        for (int i = 0; i < text.length(); i++) {
            if (i > 0) out.append(' ');
            out.append(String.format("%04x", (int) text.charAt(i)));
        }
        return out.toString();
    }

    static int bit(boolean value) {
        return value ? 1 : 0;
    }

    public static void main(String[] args) throws Exception {
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            for (String line : Files.readAllLines(Path.of(args[0]))) {
                var name = new StringBuilder();
                if (!line.equals("-")) {
                    for (String unit : line.split(" ")) name.append((char) Integer.parseInt(unit, 16));
                }
                String filtered = StringUtil.filterText(name.toString());
                out.println("n " + hex(name.toString()) + " | " + hex(filtered) + " " + bit(StringUtil.isBlank(filtered)));
            }
            for (int unit = 0; unit <= 0xFFFF; unit++) {
                char c = (char) unit;
                int whitespace = bit(StringUtil.isWhitespace(c));
                if (Character.isSurrogate(c)) {
                    out.println(String.format("u %04x %d - -", unit, whitespace));
                } else {
                    out.println(String.format("u %04x %d %d %d", unit, whitespace, bit(StringUtil.isAllowedChatCharacter(c)),
                        bit(StringUtil.isBlank(String.valueOf(c)))));
                }
            }
        }
    }
}
