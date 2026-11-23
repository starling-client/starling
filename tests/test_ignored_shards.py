"""The ignored-tests matrix names every leg of a split package, and only those."""
import unittest

from tools.ci import ignored_shards


class ShardMatrix(unittest.TestCase):
    def test_a_split_package_becomes_one_entry_per_leg(self):
        self.assertEqual(
            ignored_shards.expand(["sunrise-client", "sunrise-world"]),
            ["sunrise-client#1of2", "sunrise-client#2of2", "sunrise-world"],
        )

    def test_every_shard_is_covered_by_the_full_matrix(self):
        entries = ignored_shards.expand(ignored_shards.SHARDS)
        for package in ignored_shards.SHARDS:
            legs = ignored_shards.LEGS.get(package, 1)
            owned = [entry for entry in entries if entry.split("#")[0] == package]
            self.assertEqual(len(owned), legs)

    def test_a_package_without_legs_keeps_its_plain_name(self):
        self.assertEqual(ignored_shards.expand(["sunrise-items"]), ["sunrise-items"])


if __name__ == "__main__":
    unittest.main()
