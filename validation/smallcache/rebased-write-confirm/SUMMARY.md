# Paired results

Microseconds. Median [min, max] of 5 alternating trials per side with adjacent LMDB controls. CPU 6 only.

| Scenario / workload | Before p50 / p99 | After p50 / p99 | LMDB before p50 / p99 | LMDB after p50 / p99 | K/LMDB p50 before / after |
| --- | ---: | ---: | ---: | ---: | ---: |
| bulk/0/None/1/0/mixed-90-10 | 1.248 / 3.392 | 1.184 / 3.328 | 1.472 / 27.648 | 1.312 / 15.104 | 0.870 / 0.878 |
| bulk/0/None/1/0/recall-by-id | 1.312 / 2.688 | 1.280 / 2.624 | 1.312 / 2.368 | 1.248 / 1.792 | 1.051 / 1.024 |
| bulk/0/None/1/0/recent-for-agent | 4.992 / 10.752 | 4.736 / 8.064 | 3.904 / 7.296 | 3.776 / 5.888 | 1.254 / 1.254 |
| bulk/0/None/1/0/remember | 1.120 / 9.728 | 1.120 / 9.216 | 10.752 / 20.480 | 10.752 / 17.920 | 0.104 / 0.104 |
| bulk/0/None/1/0/update | 1.760 / 11.008 | 1.632 / 9.984 | 14.080 / 34.816 | 11.264 / 20.480 | 0.125 / 0.127 |
| bulk/0/None/1/1/mixed-90-10 | 2.112 / 704.512 | 2.016 / 720.896 | 2.240 / 1769.472 | 2.240 / 4587.520 | 0.943 / 0.924 |
| bulk/0/None/1/1/recall-by-id | 1.408 / 3.072 | 1.312 / 2.688 | 1.312 / 2.176 | 1.184 / 2.048 | 1.073 / 1.083 |
| bulk/0/None/1/1/recent-for-agent | 4.736 / 9.728 | 3.968 / 8.448 | 3.968 / 6.656 | 3.264 / 5.760 | 1.212 / 1.265 |
| bulk/0/None/1/1/remember | 622.592 / 3211.264 | 622.592 / 3276.800 | 1343.488 / 4063.232 | 1409.024 / 9961.472 | 0.450 / 0.458 |
| bulk/0/None/1/1/update | 606.208 / 1867.776 | 622.592 / 3276.800 | 1441.792 / 11010.048 | 1441.792 / 12320.768 | 0.420 / 0.442 |
| bulk/1/None/1/0/mixed-90-10 | 1.184 / 2.624 | 1.184 / 2.688 | 1.344 / 12.800 | 1.280 / 12.288 | 0.917 / 0.872 |
| bulk/1/None/1/0/recall-by-id | 1.056 / 1.600 | 1.056 / 1.760 | 1.280 / 2.016 | 1.280 / 1.888 | 0.829 / 0.825 |
| bulk/1/None/1/0/recent-for-agent | 4.736 / 8.704 | 4.608 / 9.216 | 3.648 / 7.808 | 3.648 / 5.888 | 1.298 / 1.228 |
| bulk/1/None/1/0/remember | 1.120 / 9.472 | 1.120 / 9.984 | 10.496 / 21.504 | 10.752 / 18.432 | 0.107 / 0.104 |
| bulk/1/None/1/0/update | 1.664 / 10.240 | 1.600 / 10.496 | 11.008 / 19.456 | 11.008 / 17.408 | 0.152 / 0.149 |
| bulk/1/None/1/1/mixed-90-10 | 1.984 / 753.664 | 2.304 / 901.120 | 2.304 / 1867.776 | 2.368 / 1966.080 | 0.897 / 0.919 |
| bulk/1/None/1/1/recall-by-id | 1.152 / 2.016 | 1.152 / 2.176 | 1.312 / 2.240 | 1.312 / 2.368 | 0.895 / 0.844 |
| bulk/1/None/1/1/recent-for-agent | 4.864 / 9.216 | 4.480 / 9.216 | 3.776 / 6.144 | 3.648 / 6.400 | 1.269 / 1.240 |
| bulk/1/None/1/1/remember | 638.976 / 5373.952 | 655.360 / 3342.336 | 1409.024 / 7471.104 | 1507.328 / 7077.888 | 0.447 / 0.438 |
| bulk/1/None/1/1/update | 622.592 / 3473.408 | 655.360 / 4194.304 | 1441.792 / 8912.896 | 1638.400 / 11796.480 | 0.442 / 0.433 |

## Trial ranges

| Scenario / workload | Side | p50 | p99 |
| --- | --- | ---: | ---: |
| bulk/0/None/1/0/mixed-90-10 | before-kiban | 1.248 [1.184, 1.504] | 3.392 [3.008, 4.480] |
| bulk/0/None/1/0/mixed-90-10 | before-lmdb | 1.472 [1.280, 1.728] | 27.648 [11.520, 31.744] |
| bulk/0/None/1/0/mixed-90-10 | rebased-kiban | 1.184 [1.008, 1.408] | 3.328 [2.688, 4.352] |
| bulk/0/None/1/0/mixed-90-10 | rebased-lmdb | 1.312 [1.248, 1.600] | 15.104 [11.776, 31.744] |
| bulk/0/None/1/0/recall-by-id | before-kiban | 1.312 [1.184, 1.536] | 2.688 [2.496, 3.648] |
| bulk/0/None/1/0/recall-by-id | before-lmdb | 1.312 [1.216, 1.440] | 2.368 [1.600, 2.944] |
| bulk/0/None/1/0/recall-by-id | rebased-kiban | 1.280 [1.088, 1.408] | 2.624 [2.304, 3.136] |
| bulk/0/None/1/0/recall-by-id | rebased-lmdb | 1.248 [1.152, 1.376] | 1.792 [1.664, 2.176] |
| bulk/0/None/1/0/recent-for-agent | before-kiban | 4.992 [4.736, 5.760] | 10.752 [8.064, 12.032] |
| bulk/0/None/1/0/recent-for-agent | before-lmdb | 3.904 [3.776, 4.608] | 7.296 [5.760, 9.472] |
| bulk/0/None/1/0/recent-for-agent | rebased-kiban | 4.736 [3.904, 5.504] | 8.064 [6.016, 11.776] |
| bulk/0/None/1/0/recent-for-agent | rebased-lmdb | 3.776 [3.520, 4.352] | 5.888 [5.376, 7.040] |
| bulk/0/None/1/0/remember | before-kiban | 1.120 [1.008, 1.248] | 9.728 [8.704, 11.008] |
| bulk/0/None/1/0/remember | before-lmdb | 10.752 [10.496, 12.288] | 20.480 [17.920, 24.064] |
| bulk/0/None/1/0/remember | rebased-kiban | 1.120 [0.912, 1.280] | 9.216 [7.552, 10.752] |
| bulk/0/None/1/0/remember | rebased-lmdb | 10.752 [9.984, 12.032] | 17.920 [16.896, 21.504] |
| bulk/0/None/1/0/update | before-kiban | 1.760 [1.600, 1.952] | 11.008 [9.984, 13.056] |
| bulk/0/None/1/0/update | before-lmdb | 14.080 [10.496, 17.920] | 34.816 [16.384, 43.008] |
| bulk/0/None/1/0/update | rebased-kiban | 1.632 [1.344, 1.856] | 9.984 [8.448, 13.568] |
| bulk/0/None/1/0/update | rebased-lmdb | 11.264 [10.752, 16.896] | 20.480 [16.128, 43.008] |
| bulk/0/None/1/1/mixed-90-10 | before-kiban | 2.112 [1.856, 2.624] | 704.512 [638.976, 802.816] |
| bulk/0/None/1/1/mixed-90-10 | before-lmdb | 2.240 [2.048, 2.752] | 1769.472 [1572.864, 1966.080] |
| bulk/0/None/1/1/mixed-90-10 | rebased-kiban | 2.016 [1.920, 3.008] | 720.896 [671.744, 983.040] |
| bulk/0/None/1/1/mixed-90-10 | rebased-lmdb | 2.240 [2.112, 3.008] | 4587.520 [1835.008, 12582.912] |
| bulk/0/None/1/1/recall-by-id | before-kiban | 1.408 [1.248, 2.112] | 3.072 [2.432, 4.608] |
| bulk/0/None/1/1/recall-by-id | before-lmdb | 1.312 [1.280, 1.408] | 2.176 [2.112, 3.840] |
| bulk/0/None/1/1/recall-by-id | rebased-kiban | 1.312 [1.088, 1.632] | 2.688 [2.304, 4.032] |
| bulk/0/None/1/1/recall-by-id | rebased-lmdb | 1.184 [1.088, 1.280] | 2.048 [1.632, 2.368] |
| bulk/0/None/1/1/recent-for-agent | before-kiban | 4.736 [4.096, 5.120] | 9.728 [7.168, 11.776] |
| bulk/0/None/1/1/recent-for-agent | before-lmdb | 3.968 [3.328, 4.352] | 6.656 [5.760, 7.936] |
| bulk/0/None/1/1/recent-for-agent | rebased-kiban | 3.968 [3.968, 4.992] | 8.448 [6.400, 9.472] |
| bulk/0/None/1/1/recent-for-agent | rebased-lmdb | 3.264 [3.136, 3.840] | 5.760 [5.120, 7.808] |
| bulk/0/None/1/1/remember | before-kiban | 622.592 [573.440, 704.512] | 3211.264 [1343.488, 10747.904] |
| bulk/0/None/1/1/remember | before-lmdb | 1343.488 [1310.720, 1507.328] | 4063.232 [2883.584, 9699.328] |
| bulk/0/None/1/1/remember | rebased-kiban | 622.592 [622.592, 720.896] | 3276.800 [2031.616, 11272.192] |
| bulk/0/None/1/1/remember | rebased-lmdb | 1409.024 [1343.488, 1605.632] | 9961.472 [5111.808, 12058.624] |
| bulk/0/None/1/1/update | before-kiban | 606.208 [573.440, 720.896] | 1867.776 [999.424, 6553.600] |
| bulk/0/None/1/1/update | before-lmdb | 1441.792 [1409.024, 1638.400] | 11010.048 [9699.328, 13107.200] |
| bulk/0/None/1/1/update | rebased-kiban | 622.592 [622.592, 720.896] | 3276.800 [2162.688, 3735.552] |
| bulk/0/None/1/1/update | rebased-lmdb | 1441.792 [1409.024, 1605.632] | 12320.768 [7471.104, 13631.488] |
| bulk/1/None/1/0/mixed-90-10 | before-kiban | 1.184 [1.056, 1.344] | 2.624 [2.368, 3.392] |
| bulk/1/None/1/0/mixed-90-10 | before-lmdb | 1.344 [1.152, 1.856] | 12.800 [10.496, 31.744] |
| bulk/1/None/1/0/mixed-90-10 | rebased-kiban | 1.184 [1.088, 1.248] | 2.688 [2.240, 3.008] |
| bulk/1/None/1/0/mixed-90-10 | rebased-lmdb | 1.280 [1.248, 1.568] | 12.288 [11.776, 34.816] |
| bulk/1/None/1/0/recall-by-id | before-kiban | 1.056 [0.912, 1.152] | 1.600 [1.376, 2.016] |
| bulk/1/None/1/0/recall-by-id | before-lmdb | 1.280 [1.120, 1.344] | 2.016 [1.568, 2.432] |
| bulk/1/None/1/0/recall-by-id | rebased-kiban | 1.056 [0.960, 1.184] | 1.760 [1.504, 2.112] |
| bulk/1/None/1/0/recall-by-id | rebased-lmdb | 1.280 [1.184, 1.408] | 1.888 [1.504, 2.368] |
| bulk/1/None/1/0/recent-for-agent | before-kiban | 4.736 [4.480, 5.504] | 8.704 [7.296, 10.240] |
| bulk/1/None/1/0/recent-for-agent | before-lmdb | 3.648 [3.392, 4.352] | 7.808 [6.272, 8.448] |
| bulk/1/None/1/0/recent-for-agent | rebased-kiban | 4.608 [4.032, 5.248] | 9.216 [7.296, 10.496] |
| bulk/1/None/1/0/recent-for-agent | rebased-lmdb | 3.648 [3.328, 4.096] | 5.888 [5.504, 8.448] |
| bulk/1/None/1/0/remember | before-kiban | 1.120 [1.024, 1.280] | 9.472 [8.448, 10.240] |
| bulk/1/None/1/0/remember | before-lmdb | 10.496 [9.728, 11.776] | 21.504 [16.384, 23.040] |
| bulk/1/None/1/0/remember | rebased-kiban | 1.120 [0.960, 1.280] | 9.984 [8.960, 10.496] |
| bulk/1/None/1/0/remember | rebased-lmdb | 10.752 [9.472, 11.520] | 18.432 [17.408, 22.016] |
| bulk/1/None/1/0/update | before-kiban | 1.664 [1.440, 1.984] | 10.240 [8.960, 13.824] |
| bulk/1/None/1/0/update | before-lmdb | 11.008 [9.472, 17.920] | 19.456 [14.592, 45.056] |
| bulk/1/None/1/0/update | rebased-kiban | 1.600 [1.568, 1.856] | 10.496 [9.728, 12.544] |
| bulk/1/None/1/0/update | rebased-lmdb | 11.008 [10.496, 16.896] | 17.408 [16.896, 45.056] |
| bulk/1/None/1/1/mixed-90-10 | before-kiban | 1.984 [1.824, 2.752] | 753.664 [671.744, 950.272] |
| bulk/1/None/1/1/mixed-90-10 | before-lmdb | 2.304 [2.016, 2.944] | 1867.776 [1605.632, 7077.888] |
| bulk/1/None/1/1/mixed-90-10 | rebased-kiban | 2.304 [1.792, 2.688] | 901.120 [786.432, 2293.760] |
| bulk/1/None/1/1/mixed-90-10 | rebased-lmdb | 2.368 [2.016, 3.072] | 1966.080 [1572.864, 4980.736] |
| bulk/1/None/1/1/recall-by-id | before-kiban | 1.152 [1.088, 1.472] | 2.016 [1.728, 2.752] |
| bulk/1/None/1/1/recall-by-id | before-lmdb | 1.312 [1.216, 1.568] | 2.240 [1.632, 2.752] |
| bulk/1/None/1/1/recall-by-id | rebased-kiban | 1.152 [0.960, 1.344] | 2.176 [1.600, 2.688] |
| bulk/1/None/1/1/recall-by-id | rebased-lmdb | 1.312 [1.216, 1.664] | 2.368 [1.792, 2.880] |
| bulk/1/None/1/1/recent-for-agent | before-kiban | 4.864 [4.096, 4.992] | 9.216 [8.192, 11.264] |
| bulk/1/None/1/1/recent-for-agent | before-lmdb | 3.776 [3.328, 4.096] | 6.144 [5.120, 7.168] |
| bulk/1/None/1/1/recent-for-agent | rebased-kiban | 4.480 [3.968, 4.864] | 9.216 [7.168, 10.752] |
| bulk/1/None/1/1/recent-for-agent | rebased-lmdb | 3.648 [3.136, 4.096] | 6.400 [4.864, 10.240] |
| bulk/1/None/1/1/remember | before-kiban | 638.976 [606.208, 704.512] | 5373.952 [2621.440, 9961.472] |
| bulk/1/None/1/1/remember | before-lmdb | 1409.024 [1343.488, 1572.864] | 7471.104 [3538.944, 9437.184] |
| bulk/1/None/1/1/remember | rebased-kiban | 655.360 [622.592, 704.512] | 3342.336 [2752.512, 5898.240] |
| bulk/1/None/1/1/remember | rebased-lmdb | 1507.328 [1409.024, 1572.864] | 7077.888 [3735.552, 15204.352] |
| bulk/1/None/1/1/update | before-kiban | 622.592 [606.208, 737.280] | 3473.408 [3276.800, 4325.376] |
| bulk/1/None/1/1/update | before-lmdb | 1441.792 [1409.024, 1572.864] | 8912.896 [7077.888, 13631.488] |
| bulk/1/None/1/1/update | rebased-kiban | 655.360 [622.592, 737.280] | 4194.304 [1507.328, 4980.736] |
| bulk/1/None/1/1/update | rebased-lmdb | 1638.400 [1409.024, 2293.760] | 11796.480 [10485.760, 15728.640] |
