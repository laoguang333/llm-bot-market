import unittest

from study_v6_population import maximum_rise


class ChronologicalRiseTests(unittest.TestCase):
    def test_decline_is_not_a_rise(self):
        self.assertEqual(maximum_rise([(0, 200), (1, 100)], 100), (1.0, None))

    def test_uses_ticks_not_row_count(self):
        self.assertEqual(maximum_rise([(0, 100), (101, 300)], 100), (1.0, None))

    def test_inclusive_window_and_expiring_minimum(self):
        self.assertEqual(maximum_rise([(0, 50), (10, 100), (100, 200), (110, 500)], 100),
                         (5.0, [10, 110, 100, 500]))

    def test_empty_or_single_print(self):
        self.assertEqual(maximum_rise([], 100), (1.0, None))
        self.assertEqual(maximum_rise([(5, 100)], 100), (1.0, None))


if __name__ == '__main__':
    unittest.main()
