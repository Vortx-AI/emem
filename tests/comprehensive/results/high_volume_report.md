# High-volume questions — what emem says about what users actually ask

Filtered to questions whose phrasing matches the high-volume patterns AI agents see most: 'should I buy here', 'is the air safe', 'will my home flood', 'is X a climate haven', 'how bad is the smoke today'. These are the AI-agent queries that drive real protocol traffic, not GIS-analyst queries.

Sample size: **39** very_high + high volume questions.

| Vol | Sev | Question | Place | Top match | Facts | Materialize | Lat | Pass |
|---|---|---|---|---|---|---|---|---|
| high | 5 | wildfire near maui kula upcountry | Kula, Maui, Hawaii | `fire_burn_severity` (0.653) | 32 | 32 | 30.1s | OK |
| high | 5 | valencia spain floods october 2024 inundation exte | Valencia, Spain | `flood_history_long_term` (0.695) | 19 | 15 | 24.7s | OK |
| high | 5 | derna libya dam collapse flood damage | Derna, Libya | `flood_water_event_window` (0.683) | 19 | 19 | 20.9s | OK |
| high | 5 | hajj heatstroke risk mecca | Mecca, Saudi Arabia | `parametric_insurance` (0.681) | 31 | 31 | 28.0s | OK |
| high | 5 | horn of africa drought somalia food crisis | Baidoa, Somalia | `parametric_insurance` (0.662) | 28 | 28 | 33.6s | OK |
| high | 5 | will tuvalu still exist in 2050 | Funafuti, Tuvalu | `weather_now` (0.584) | 20 | 22 | 23.9s | OK |
| high | 5 | sinking lands and sea rise male maldives | Malé, Maldives | `elevation_global_topobathy` (0.615) | 16 | 16 | 17.9s | OK |
| high | 4 | is it dumb to buy a house in paradise california a | Paradise, California | `flood_risk_composite` (0.68) | 25 | 25 | 28.1s | OK |
| high | 4 | is duluth minnesota actually a climate refuge | Duluth, Minnesota | `weather_now` (0.609) | 24 | 25 | 27.8s | OK |
| high | 4 | is buffalo new york a good climate haven for retir | Buffalo, New York | `urban_livability` (0.63) | 27 | 27 | 27.6s | OK |
| high | 4 | bushfire risk blue mountains nsw this summer | Katoomba, New South Wales | `parametric_insurance` (0.661) | 24 | 24 | 28.9s | OK |
| high | 4 | how often does chennai velachery actually waterlog | Velachery, Chennai, India | `flood_water_event_window` (0.67) | 19 | 6 | 10.6s | OK |
| high | 4 | how hot are nights in karachi getting these days | Karachi, Pakistan | `weather_now` (0.666) | 30 | 30 | 28.4s | OK |
| high | 4 | is hanoi air pollution worse than beijing now | Ba Dinh, Hanoi, Vietnam | `public_health` (0.689) | 30 | 31 | 29.9s | OK |
| high | 4 | insurer non-renewals santa rosa california wildfir | Santa Rosa, California | `fire_burn_severity` (0.593) | 25 | 25 | 25.4s | OK |
| high | 4 | how is the maize harvest looking in the us corn be | Ames, Iowa | `agriculture` (0.71) | 33 | 34 | 32.5s | OK |
| high | 4 | is bengaluru going to run out of water like 2024 a | Bellandur, Bangalore, Ind | `flood_water_event_window` (0.678) | 19 | 19 | 20.6s | OK |
| high | 4 | is the amazon nearing the tipping point in para st | Altamira, Pará, Brazil | `esg` (0.655) | 24 | 25 | 32.6s | OK |
| high | 3 | is asheville north carolina still a climate haven  | Asheville, North Carolina | `vegetation_condition` (0.048) | 28 | 28 | 29.1s | OK |
| high | 3 | how walkable is the polanco neighborhood mexico ci | Polanco, Mexico City | `urban_livability` (0.676) | 17 | 17 | 19.6s | OK |
| high | 3 | green space and tree cover in koramangala bangalor | Koramangala, Bangalore, I | `urban_livability` (0.693) | 24 | 24 | 27.1s | OK |
| high | 3 | thailand monsoon flood risk koh samui october | Koh Samui, Thailand | `flood_risk_composite` (0.222) | 19 | 19 | 20.7s | OK |
| high | 3 | which is more flood prone gurgaon or noida for buy | DLF Phase 5, Gurgaon, Ind | `flood_risk_composite` (0.725) | 19 | 19 | 20.8s | OK |
| high | 3 | i'm worried about the climate where i live can you | Edinburgh, Scotland | `weather_now` (0.754) | 28 | 28 | 26.4s | OK |
| high | 3 | is my hometown going to be uninhabitable | Basra, Iraq | `urban_livability` (0.747) | 16 | 16 | 15.8s | OK |
| high | 2 | is my rooftop in jaipur worth installing solar pan | Malviya Nagar, Jaipur, In | — | 0 | 0 | 150.3s | FAIL |
| very_high | 5 | is my home in the palisades likely to burn this fi | Pacific Palisades, Los An | `fire_burn_severity` (0.73) | 26 | 26 | 21.7s | OK |
| very_high | 5 | is dubai still flooded after the april rains | Dubai, United Arab Emirat | `flood_history_long_term` (0.706) | 19 | 19 | 20.4s | OK |
| very_high | 5 | how dangerous is the heat dome in seville this wee | Seville, Spain | `vegetation_condition` (0.059) | 32 | 12 | 11.2s | OK |
| very_high | 5 | heatwave delhi may 2026 health risk for kids | Connaught Place, New Delh | `public_health` (0.182) | 29 | 30 | 28.0s | OK |
| very_high | 4 | should i buy a flat in lower parel mumbai or is it | Lower Parel, Mumbai, Indi | `flood_risk_composite` (0.827) | 15 | 0 | 2.9s | OK |
| very_high | 4 | is gurgaon sector 65 safe from waterlogging in mon | Sector 65, Gurgaon, India | `flood_water_event_window` (0.7) | 19 | 0 | 2.9s | OK |
| very_high | 4 | thinking of buying a beach house in tampa florida  | Tampa, Florida | `flood_risk_composite` (0.742) | 19 | 1 | 6.9s | OK |
| very_high | 4 | how bad is wildfire smoke in toronto today from qu | Toronto, Canada | `public_health` (0.694) | 24 | 24 | 18.9s | OK |
| very_high | 4 | smoke from canadian wildfires new york city | Manhattan, New York | `fire_burn_severity` (0.656) | 33 | 33 | 26.7s | OK |
| very_high | 4 | is the air safe to walk outside in lahore today | Gulberg, Lahore, Pakistan | `public_health` (0.319) | 23 | 23 | 19.6s | OK |
| very_high | 4 | smog in delhi gurgaon noida how bad is it | Sector 18, Noida, India | `public_health` (0.171) | 29 | 29 | 23.1s | OK |
| very_high | 4 | why is home insurance unaffordable in cape coral f | Cape Coral, Florida | `real_estate` (0.666) | 22 | 22 | 22.8s | OK |
| very_high | 3 | is it safe to visit bali during volcanic ash from  | Ubud, Bali, Indonesia | `public_health` (0.666) | 33 | 33 | 29.0s | OK |

## Headline numbers

- High-volume routing accuracy: **38/39 (97%)**
- High-volume questions returning >=1 signed fact: **38/39**
- Average latency: **26.04s**