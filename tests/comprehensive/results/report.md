# emem comprehensive consumer evaluation

- Endpoint: `https://emem.dev`
- Started: 2026-05-08T15:07:22+00:00
- Finished: 2026-05-08T15:51:58+00:00
- Questions: **105**
- Routing-correct: **100/105** (95%)
- Volume-weighted routing accuracy: **96.4%**  (weights very_high=4, high=3, medium=2, low=1)
- HTTP 200: 104/105
- Place resolved: 104/105
- Returned >=1 fact: 104/105
- Severity-weighted miss cost (sum of severity for failures, lower is better): **11**
- Latency: avg 27.28s, p50 26.94s, p95 39.15s

## How to read this

**Routing-correct** means the responder's `topic_routing.matched_topics` (or `algorithms_for_question.topic`) overlapped the expected topic set, **or** the question was out_of_scope and we expected nothing. This measures whether the LLM-facing router on `/v1/ask` is sending the question to the right Earth-observation primitives. It does **not** measure whether materialization succeeded or whether the answer is scientifically correct — it measures whether the protocol *understood the question*.

**Place resolved** means the geocoder returned a `cell64`. **Facts** count signed facts attached to the response — usually attested-band reads from the local cache. **Materialize attempts** count cold-band fan-outs, which can be slow on first call.

## By domain

| Domain | Pass | Total | % | Avg latency | Avg facts | Avg materialize |
|---|---|---|---|---|---|---|
| real_estate | 8 | 8 | 100% | 18.4s | 22.4 | 15.9 |
| new_locations_under_observed | 8 | 8 | 100% | 29.0s | 26.2 | 26.6 |
| wildfire | 7 | 7 | 100% | 26.6s | 29.1 | 29.3 |
| flooding | 7 | 7 | 100% | 19.6s | 19.0 | 16.6 |
| food_security | 7 | 7 | 100% | 32.6s | 31.0 | 31.4 |
| heat_health | 6 | 6 | 100% | 23.0s | 28.3 | 25.2 |
| climate_migration | 4 | 5 | 80% | 23.4s | 26.0 | 21.8 |
| air_quality | 5 | 5 | 100% | 26.7s | 28.0 | 28.2 |
| insurance | 5 | 5 | 100% | 24.1s | 25.0 | 22.8 |
| water_security | 4 | 5 | 80% | 25.1s | 20.0 | 20.2 |
| forest_carbon | 5 | 5 | 100% | 49.6s | 28.4 | 29.6 |
| energy_transition | 4 | 5 | 80% | 52.6s | 20.2 | 20.6 |
| travel_safety | 5 | 5 | 100% | 24.3s | 24.8 | 24.8 |
| coastal | 5 | 5 | 100% | 21.7s | 19.2 | 19.6 |
| glacier_polar | 4 | 4 | 100% | 18.7s | 19.0 | 20.2 |
| urban_planning | 4 | 4 | 100% | 24.7s | 25.8 | 26.0 |
| esg_due_diligence | 4 | 4 | 100% | 29.9s | 30.8 | 31.8 |
| new_question_type_temporal | 3 | 3 | 100% | 29.9s | 25.7 | 26.0 |
| new_question_type_natural_language | 3 | 3 | 100% | 22.8s | 23.0 | 23.0 |
| new_question_type_comparative | 2 | 2 | 100% | 24.2s | 25.0 | 25.0 |
| out_of_scope_check | 0 | 2 | 0% | 28.7s | 24.5 | 25.0 |

## By volume signal (where users actually ask)

Higher volume = the protocol must answer this *well* because it's what users ask AI agents most.

| Volume | Pass | Total | % | Sample |
|---|---|---|---|---|
| very_high | 13 | 13 | 100% | _should i buy a flat in lower parel mumbai or is it flood pro_ |
| high | 25 | 26 | 96% | _is my rooftop in jaipur worth installing solar panels_ |
| medium | 55 | 57 | 96% | _climate safe places in portugal away from wildfire_ |
| low | 7 | 9 | 77% | _who won the 2024 election_ |

## By region (geographic equity of coverage)

| Region | Pass | Total | % | Avg latency | Place-resolve % |
|---|---|---|---|---|---|
| North America | 21 | 22 | 95% | 24.3s | 100% |
| South Asia | 19 | 20 | 95% | 28.7s | 95% |
| Europe | 14 | 15 | 93% | 24.3s | 100% |
| East/SE Asia | 12 | 12 | 100% | 24.9s | 100% |
| Africa | 9 | 9 | 100% | 43.4s | 100% |
| Middle East | 8 | 8 | 100% | 22.7s | 100% |
| South America | 7 | 7 | 100% | 29.8s | 100% |
| Other | 4 | 6 | 66% | 28.9s | 100% |
| Oceania | 4 | 4 | 100% | 26.4s | 100% |
| Polar | 2 | 2 | 100% | 17.2s | 100% |

## Top routing failures (sorted by severity x volume — these are the ones to fix first)

| Sev | Volume | Domain | Question | Place | Matched (if any) | Out-of-scope? |
|---|---|---|---|---|---|---|
| 4 | medium | water_security | aral sea recovery uzbekistan kazakhstan | Muynak, Uzbekistan | `flood_water_event_window` | False |
| 3 | medium | climate_migration | climate safe places in portugal away from wildfire | Coimbra, Portugal | `urban_livability` | False |
| 2 | high | energy_transition | is my rooftop in jaipur worth installing solar panels | Malviya Nagar, Jaipur, India | `—` | None |
| 1 | low | out_of_scope_check | who won the 2024 election | Washington, DC | `weather_now` | False |
| 1 | low | out_of_scope_check | what is the meaning of life | Earth | `vegetation_condition` | False |

## All questions

| ID | Vol | Sev | Domain | Question | Routed via | Top match | Pass | Facts | Lat |
|---|---|---|---|---|---|---|---|---|---|
| 101 | very_high | 4 | real_estate | should i buy a flat in lower parel mumbai or is  | ort | `flood_risk_composite` (0.827) | OK | 15 | 2.9s |
| 102 | very_high | 4 | real_estate | is gurgaon sector 65 safe from waterlogging in m | ort | `flood_water_event_window` (0.7) | OK | 19 | 2.9s |
| 103 | very_high | 4 | real_estate | thinking of buying a beach house in tampa florid | ort | `flood_risk_composite` (0.742) | OK | 19 | 6.9s |
| 104 | high | 3 | real_estate | is asheville north carolina still a climate have | ort | `vegetation_condition` (0.048) | OK | 28 | 29.1s |
| 105 | high | 3 | real_estate | how walkable is the polanco neighborhood mexico  | ort | `urban_livability` (0.676) | OK | 17 | 19.6s |
| 106 | high | 3 | real_estate | green space and tree cover in koramangala bangal | ort | `urban_livability` (0.693) | OK | 24 | 27.1s |
| 107 | high | 4 | real_estate | is it dumb to buy a house in paradise california | ort | `flood_risk_composite` (0.68) | OK | 25 | 28.1s |
| 108 | medium | 4 | real_estate | sea level rise threat to flats in bandra reclama | ort | `flood_risk_composite` (0.665) | OK | 32 | 30.8s |
| 110 | high | 4 | climate_migration | is duluth minnesota actually a climate refuge | ort | `weather_now` (0.609) | OK | 24 | 27.8s |
| 111 | high | 4 | climate_migration | is buffalo new york a good climate haven for ret | ort | `urban_livability` (0.63) | OK | 27 | 27.6s |
| 112 | medium | 3 | climate_migration | climate safe places in portugal away from wildfi | ort | `urban_livability` (0.682) | FAIL | 32 | 34.6s |
| 113 | medium | 4 | climate_migration | how livable will phoenix be in 2050 with extreme | ort | `urban_livability` (0.658) | OK | 23 | 3.2s |
| 114 | medium | 3 | climate_migration | is the great lakes region the best place to esca | ort | `weather_now` (0.621) | OK | 24 | 23.9s |
| 120 | very_high | 5 | wildfire | is my home in the palisades likely to burn this  | ort | `fire_burn_severity` (0.73) | OK | 26 | 21.7s |
| 121 | very_high | 4 | wildfire | how bad is wildfire smoke in toronto today from  | ort | `public_health` (0.694) | OK | 24 | 18.9s |
| 122 | very_high | 4 | wildfire | smoke from canadian wildfires new york city | ort | `fire_burn_severity` (0.656) | OK | 33 | 26.7s |
| 123 | high | 5 | wildfire | wildfire near maui kula upcountry | ort | `fire_burn_severity` (0.653) | OK | 32 | 30.1s |
| 124 | high | 4 | wildfire | bushfire risk blue mountains nsw this summer | ort | `parametric_insurance` (0.661) | OK | 24 | 28.9s |
| 125 | medium | 4 | wildfire | how much of rhodes greece burned in the 2023 fir | ort | `fire_burn_severity` (0.704) | OK | 29 | 28.8s |
| 126 | medium | 4 | wildfire | burn scar from valparaiso chile fire 2024 | ort | `fire_burn_severity` (0.833) | OK | 36 | 30.9s |
| 130 | very_high | 5 | flooding | is dubai still flooded after the april rains | ort | `flood_history_long_term` (0.706) | OK | 19 | 20.4s |
| 131 | high | 5 | flooding | valencia spain floods october 2024 inundation ex | ort | `flood_history_long_term` (0.695) | OK | 19 | 24.7s |
| 132 | high | 5 | flooding | derna libya dam collapse flood damage | ort | `flood_water_event_window` (0.683) | OK | 19 | 20.9s |
| 133 | high | 4 | flooding | how often does chennai velachery actually waterl | ort | `flood_water_event_window` (0.67) | OK | 19 | 10.6s |
| 134 | medium | 4 | flooding | is khartoum at risk from nile flooding this year | ort | `flood_risk_composite` (0.689) | OK | 19 | 20.2s |
| 135 | medium | 4 | flooding | porto alegre brazil flood recovery 2024 | ort | `flood_history_long_term` (0.665) | OK | 19 | 19.8s |
| 136 | medium | 3 | flooding | is venice still sinking and how often does san m | ort | `flood_history_long_term` (0.709) | OK | 19 | 20.3s |
| 140 | very_high | 5 | heat_health | how dangerous is the heat dome in seville this w | ort | `vegetation_condition` (0.059) | OK | 32 | 11.2s |
| 141 | very_high | 5 | heat_health | heatwave delhi may 2026 health risk for kids | ort | `public_health` (0.182) | OK | 29 | 28.0s |
| 142 | high | 4 | heat_health | how hot are nights in karachi getting these days | ort | `weather_now` (0.666) | OK | 30 | 28.4s |
| 143 | high | 5 | heat_health | hajj heatstroke risk mecca | ort | `parametric_insurance` (0.681) | OK | 31 | 28.0s |
| 144 | medium | 3 | heat_health | which neighborhoods in athens have worst urban h | ort | `urban_livability` (0.293) | OK | 25 | 21.7s |
| 145 | medium | 3 | heat_health | is paris ile-de-france going to be uninhabitable | ort | `urban_livability` (0.67) | OK | 23 | 20.8s |
| 150 | very_high | 4 | air_quality | is the air safe to walk outside in lahore today | ort | `public_health` (0.319) | OK | 23 | 19.6s |
| 151 | very_high | 4 | air_quality | smog in delhi gurgaon noida how bad is it | ort | `public_health` (0.171) | OK | 29 | 23.1s |
| 152 | high | 4 | air_quality | is hanoi air pollution worse than beijing now | ort | `public_health` (0.689) | OK | 30 | 29.9s |
| 153 | medium | 3 | air_quality | pm2.5 dhaka bangladesh winter | ort | `public_health` (0.172) | OK | 28 | 28.3s |
| 154 | medium | 3 | air_quality | saharan dust storm air quality canary islands | ort | `public_health` (0.7) | OK | 30 | 32.4s |
| 160 | medium | 4 | insurance | hurricane parametric trigger probability gulf co | ort | `parametric_insurance` (0.77) | OK | 20 | 26.9s |
| 161 | very_high | 4 | insurance | why is home insurance unaffordable in cape coral | ort | `real_estate` (0.666) | OK | 22 | 22.8s |
| 162 | high | 4 | insurance | insurer non-renewals santa rosa california wildf | ort | `fire_burn_severity` (0.593) | OK | 25 | 25.4s |
| 163 | medium | 3 | insurance | drought index payout for cotton farmers australi | ort | `parametric_insurance` (0.702) | OK | 33 | 35.4s |
| 164 | medium | 3 | insurance | typhoon parametric coverage philippines luzon | ort | `parametric_insurance` (0.706) | OK | 25 | 10.1s |
| 170 | high | 4 | food_security | how is the maize harvest looking in the us corn  | ort | `agriculture` (0.71) | OK | 33 | 32.5s |
| 171 | high | 5 | food_security | horn of africa drought somalia food crisis | ort | `parametric_insurance` (0.662) | OK | 28 | 33.6s |
| 172 | medium | 3 | food_security | coffee rust risk minas gerais brazil | ort | `agriculture` (0.64) | OK | 38 | 39.1s |
| 173 | medium | 4 | food_security | groundwater stress on rice farms central valley  | ort | `soil_intelligence` (0.693) | OK | 40 | 33.6s |
| 174 | medium | 3 | food_security | vineyard drought stress bordeaux this season | ort | `agriculture` (0.182) | OK | 28 | 29.5s |
| 175 | medium | 3 | food_security | tea garden vigor assam india | ort | `agriculture` (0.643) | OK | 31 | 34.7s |
| 176 | medium | 4 | food_security | rice paddies inundated by floods in sindh pakist | ort | `flood_history_long_term` (0.727) | OK | 19 | 25.3s |
| 180 | high | 4 | water_security | is bengaluru going to run out of water like 2024 | ort | `flood_water_event_window` (0.678) | OK | 19 | 20.6s |
| 181 | medium | 4 | water_security | reservoir levels barcelona drought | ort | `flood_water_event_window` (0.441) | OK | 19 | 24.4s |
| 182 | medium | 4 | water_security | is the great salt lake going to disappear | ort | `flood_water_event_window` (0.68) | OK | 19 | 35.5s |
| 183 | medium | 4 | water_security | aral sea recovery uzbekistan kazakhstan | ort | `flood_water_event_window` (0.585) | FAIL | 25 | 27.6s |
| 184 | medium | 3 | water_security | dam levels itaipu paraguay brazil | ort | `flood_water_event_window` (0.638) | OK | 18 | 17.3s |
| 190 | high | 4 | forest_carbon | is the amazon nearing the tipping point in para  | ort | `esg` (0.655) | OK | 24 | 32.6s |
| 191 | medium | 4 | forest_carbon | oil palm expansion deforestation papua indonesia | ort | `esg` (0.678) | OK | 28 | 29.8s |
| 192 | medium | 3 | forest_carbon | great green wall progress sahel senegal | ort | `vegetation_condition` (0.62) | OK | 34 | 39.4s |
| 193 | medium | 3 | forest_carbon | peatland carbon stock central kalimantan | ort | `carbon_credits` (0.715) | OK | 31 | 30.1s |
| 194 | medium | 4 | forest_carbon | congo basin forest loss kisangani | ort | `esg` (0.657) | OK | 25 | 116.2s |
| 200 | high | 2 | energy_transition | is my rooftop in jaipur worth installing solar p | None | — | FAIL | 0 | 150.3s |
| 201 | medium | 2 | energy_transition | utility solar farm siting potential atacama chil | ort | `esg` (0.641) | OK | 31 | 39.8s |
| 202 | medium | 2 | energy_transition | offshore wind potential east coast vietnam | ort | `weather_now` (0.649) | OK | 20 | 21.2s |
| 203 | medium | 2 | energy_transition | wind capacity factor north sea netherlands | ort | `weather_now` (0.702) | OK | 30 | 34.4s |
| 204 | low | 3 | energy_transition | snowpack hydro generation pacific northwest | ort | `snow` (0.186) | OK | 20 | 17.3s |
| 210 | very_high | 3 | travel_safety | is it safe to visit bali during volcanic ash fro | ort | `public_health` (0.666) | OK | 33 | 29.0s |
| 211 | high | 3 | travel_safety | thailand monsoon flood risk koh samui october | ort | `flood_risk_composite` (0.222) | OK | 19 | 20.7s |
| 212 | medium | 3 | travel_safety | sahara dust haboob risk marrakech | ort | `public_health` (0.693) | OK | 30 | 27.1s |
| 213 | medium | 3 | travel_safety | is iceland safe to drive in november ring road | ort | `urban_livability` (0.58) | OK | 23 | 20.5s |
| 214 | medium | 3 | travel_safety | avalanche risk chamonix france ski season | ort | `snow` (0.678) | OK | 19 | 24.0s |
| 220 | medium | 4 | glacier_polar | is thwaites glacier collapse imminent | ort | `flood_risk_composite` (0.633) | OK | 18 | 14.7s |
| 221 | medium | 4 | glacier_polar | how fast is the gangotri glacier retreating | ort | `weather_now` (0.63) | OK | 20 | 20.6s |
| 222 | medium | 4 | glacier_polar | swiss alps glacier loss aletsch | ort | `snow` (0.632) | OK | 20 | 20.0s |
| 223 | medium | 4 | glacier_polar | greenland ice sheet melt jakobshavn | ort | `snow` (0.643) | OK | 18 | 19.7s |
| 230 | high | 5 | coastal | will tuvalu still exist in 2050 | ort | `weather_now` (0.584) | OK | 20 | 23.9s |
| 231 | high | 5 | coastal | sinking lands and sea rise male maldives | ort | `elevation_global_topobathy` (0.615) | OK | 16 | 17.9s |
| 232 | medium | 4 | coastal | coastal erosion lagos nigeria victoria island | ort | `flood_water_event_window` (0.632) | OK | 22 | 27.0s |
| 233 | medium | 4 | coastal | sea rise threat to alexandria egypt | ort | `flood_water_event_window` (0.643) | OK | 19 | 20.0s |
| 234 | low | 3 | coastal | sand loss outer banks north carolina rodanthe | ort | `parametric_insurance` (0.661) | OK | 19 | 19.9s |
| 240 | medium | 2 | urban_planning | 15 minute city scoring barcelona eixample | ort | `urban_livability` (0.615) | OK | 30 | 22.3s |
| 241 | medium | 2 | urban_planning | tree canopy equity south side chicago | ort | `urban_livability` (0.704) | OK | 32 | 30.8s |
| 242 | medium | 3 | urban_planning | informal settlement growth dharavi mumbai | ort | `built_up_human_geography` (0.244) | OK | 25 | 24.8s |
| 243 | medium | 2 | urban_planning | how dense is kowloon hong kong actually | ort | `built_up_human_geography` (0.631) | OK | 16 | 20.9s |
| 250 | medium | 3 | esg_due_diligence | environmental impact zone around foxconn zhengzh | ort | `esg` (0.728) | OK | 32 | 33.3s |
| 251 | medium | 4 | esg_due_diligence | deforestation footprint cobalt mining katanga dr | ort | `esg` (0.72) | OK | 27 | 27.2s |
| 252 | medium | 3 | esg_due_diligence | water stress around lithium mine salar de atacam | ort | `weather_now` (0.694) | OK | 34 | 28.9s |
| 253 | low | 3 | esg_due_diligence | steel mill emissions plume tata jamshedpur | ort | `public_health` (0.674) | OK | 30 | 30.4s |
| 260 | high | 3 | new_question_type_comparative | which is more flood prone gurgaon or noida for b | ort | `flood_risk_composite` (0.725) | OK | 19 | 20.8s |
| 261 | medium | 3 | new_question_type_comparative | is austin or denver better for climate over the  | ort | `weather_now` (0.651) | OK | 31 | 27.7s |
| 270 | medium | 3 | new_question_type_temporal | how has tree cover changed in singapore over the | ort | `esg` (0.677) | OK | 28 | 30.5s |
| 271 | medium | 4 | new_question_type_temporal | trend of summer maximum temperatures in baghdad  | ort | `weather_now` (0.642) | OK | 26 | 29.4s |
| 272 | medium | 3 | new_question_type_temporal | how is built up area expanding around addis abab | ort | `built_up_human_geography` (0.72) | OK | 23 | 29.9s |
| 280 | high | 3 | new_question_type_natural_language | i'm worried about the climate where i live can y | ort | `weather_now` (0.754) | OK | 28 | 26.4s |
| 281 | high | 3 | new_question_type_natural_language | is my hometown going to be uninhabitable | ort | `urban_livability` (0.747) | OK | 16 | 15.8s |
| 282 | medium | 3 | new_question_type_natural_language | what should i be scared of climate-wise here | ort | `flood_risk_composite` (0.661) | OK | 25 | 26.2s |
| 290 | low | 1 | out_of_scope_check | who won the 2024 election | ort | `weather_now` (0.589) | FAIL | 26 | 28.4s |
| 291 | low | 1 | out_of_scope_check | what is the meaning of life | ort | `vegetation_condition` (0.584) | FAIL | 23 | 28.9s |
| 300 | medium | 3 | new_locations_under_observed | flood risk in pacific island nation kiribati | ort | `flood_risk_composite` (0.227) | OK | 18 | 17.6s |
| 301 | medium | 3 | new_locations_under_observed | vegetation health madagascar dry south | ort | `vegetation_condition` (0.447) | OK | 30 | 37.9s |
| 302 | medium | 3 | new_locations_under_observed | sahel rainfall trends timbuktu mali | ort | `weather_now` (0.694) | OK | 38 | 39.6s |
| 303 | medium | 3 | new_locations_under_observed | snow cover mongolia ulaanbaatar this winter | ort | `snow` (0.233) | OK | 18 | 16.9s |
| 304 | low | 3 | new_locations_under_observed | earthquake vulnerability and topography port-au- | ort | `topography` (0.185) | OK | 21 | 26.1s |
| 305 | low | 3 | new_locations_under_observed | oil spill recovery delta niger | ort | `flood_water_event_window` (0.613) | OK | 25 | 40.0s |
| 306 | low | 3 | new_locations_under_observed | glacial lake outburst risk sikkim india | ort | `flood_water_event_window` (0.675) | OK | 23 | 26.1s |
| 307 | low | 3 | new_locations_under_observed | dust bowl risk aralkum desert | ort | `parametric_insurance` (0.668) | OK | 37 | 28.1s |