# Regional climate-question rollup

How well does emem serve climate questions across regions?  Coverage equity matters: a protocol that answers Florida flood questions but stalls on Bangladesh delta questions is unfit for the global agent ecosystem.

## North America  (22 questions)

- Routing accuracy: **21/22 (95%)**
- Geocoder resolved: 22/22
- Returned >=1 signed fact: 22/22
- Avg latency: 24.30s

| Sev | Domain | Question | Place | Pass |
|---|---|---|---|---|
| 5 | wildfire | is my home in the palisades likely to burn this fi | Pacific Palisades, Los Angeles | OK |
| 4 | climate_migration | is duluth minnesota actually a climate refuge | Duluth, Minnesota | OK |
| 4 | climate_migration | is buffalo new york a good climate haven for retir | Buffalo, New York | OK |
| 4 | climate_migration | how livable will phoenix be in 2050 with extreme h | Phoenix, Arizona | OK |
| 4 | food_security | how is the maize harvest looking in the us corn be | Ames, Iowa | OK |
| 4 | food_security | groundwater stress on rice farms central valley ca | Fresno, California | OK |
| 4 | insurance | hurricane parametric trigger probability gulf coas | Lafayette, Louisiana | OK |
| 4 | insurance | why is home insurance unaffordable in cape coral f | Cape Coral, Florida | OK |
| 4 | insurance | insurer non-renewals santa rosa california wildfir | Santa Rosa, California | OK |
| 4 | real_estate | thinking of buying a beach house in tampa florida  | Tampa, Florida | OK |
| 4 | real_estate | is it dumb to buy a house in paradise california a | Paradise, California | OK |
| 4 | water_security | is the great salt lake going to disappear | Antelope Island, Utah | OK |
| 4 | wildfire | how bad is wildfire smoke in toronto today from qu | Toronto, Canada | OK |
| 4 | wildfire | smoke from canadian wildfires new york city | Manhattan, New York | OK |
| 3 | climate_migration | is the great lakes region the best place to escape | Cleveland, Ohio | OK |
| 3 | coastal | sand loss outer banks north carolina rodanthe | Rodanthe, North Carolina | OK |
| 3 | energy_transition | snowpack hydro generation pacific northwest | Wenatchee, Washington | OK |
| 3 | new_question_type_comparative | is austin or denver better for climate over the ne | Austin, Texas | OK |
| 3 | real_estate | is asheville north carolina still a climate haven  | Asheville, North Carolina | OK |
| 3 | real_estate | how walkable is the polanco neighborhood mexico ci | Polanco, Mexico City | OK |
| 2 | urban_planning | tree canopy equity south side chicago | Englewood, Chicago, Illinois | OK |
| 1 | out_of_scope_check | who won the 2024 election | Washington, DC | FAIL |

## South Asia  (20 questions)

- Routing accuracy: **19/20 (95%)**
- Geocoder resolved: 19/20
- Returned >=1 signed fact: 19/20
- Avg latency: 28.66s

| Sev | Domain | Question | Place | Pass |
|---|---|---|---|---|
| 5 | coastal | sinking lands and sea rise male maldives | Malé, Maldives | OK |
| 5 | heat_health | heatwave delhi may 2026 health risk for kids | Connaught Place, New Delhi, In | OK |
| 4 | air_quality | is the air safe to walk outside in lahore today | Gulberg, Lahore, Pakistan | OK |
| 4 | air_quality | smog in delhi gurgaon noida how bad is it | Sector 18, Noida, India | OK |
| 4 | flooding | how often does chennai velachery actually waterlog | Velachery, Chennai, India | OK |
| 4 | food_security | rice paddies inundated by floods in sindh pakistan | Larkana, Sindh, Pakistan | OK |
| 4 | glacier_polar | how fast is the gangotri glacier retreating | Gangotri Glacier, Uttarakhand, | OK |
| 4 | heat_health | how hot are nights in karachi getting these days | Karachi, Pakistan | OK |
| 4 | real_estate | should i buy a flat in lower parel mumbai or is it | Lower Parel, Mumbai, India | OK |
| 4 | real_estate | is gurgaon sector 65 safe from waterlogging in mon | Sector 65, Gurgaon, India | OK |
| 4 | real_estate | sea level rise threat to flats in bandra reclamati | Bandra Reclamation, Mumbai, In | OK |
| 4 | water_security | is bengaluru going to run out of water like 2024 a | Bellandur, Bangalore, India | OK |
| 3 | air_quality | pm2.5 dhaka bangladesh winter | Gulshan, Dhaka, Bangladesh | OK |
| 3 | esg_due_diligence | steel mill emissions plume tata jamshedpur | Jamshedpur, Jharkhand, India | OK |
| 3 | food_security | tea garden vigor assam india | Jorhat, Assam, India | OK |
| 3 | new_locations_under_observed | glacial lake outburst risk sikkim india | South Lhonak Lake, Sikkim, Ind | OK |
| 3 | new_question_type_comparative | which is more flood prone gurgaon or noida for buy | DLF Phase 5, Gurgaon, India | OK |
| 3 | real_estate | green space and tree cover in koramangala bangalor | Koramangala, Bangalore, India | OK |
| 3 | urban_planning | informal settlement growth dharavi mumbai | Dharavi, Mumbai, India | OK |
| 2 | energy_transition | is my rooftop in jaipur worth installing solar pan | Malviya Nagar, Jaipur, India | FAIL |

## Europe  (15 questions)

- Routing accuracy: **14/15 (93%)**
- Geocoder resolved: 15/15
- Returned >=1 signed fact: 15/15
- Avg latency: 24.25s

| Sev | Domain | Question | Place | Pass |
|---|---|---|---|---|
| 5 | flooding | valencia spain floods october 2024 inundation exte | Valencia, Spain | OK |
| 5 | heat_health | how dangerous is the heat dome in seville this wee | Seville, Spain | OK |
| 4 | glacier_polar | swiss alps glacier loss aletsch | Aletsch Glacier, Switzerland | OK |
| 4 | water_security | reservoir levels barcelona drought | Sau Reservoir, Catalonia, Spai | OK |
| 4 | wildfire | how much of rhodes greece burned in the 2023 fires | Rhodes, Greece | OK |
| 3 | climate_migration | climate safe places in portugal away from wildfire | Coimbra, Portugal | FAIL |
| 3 | flooding | is venice still sinking and how often does san mar | Piazza San Marco, Venice, Ital | OK |
| 3 | food_security | vineyard drought stress bordeaux this season | Saint-Émilion, France | OK |
| 3 | heat_health | which neighborhoods in athens have worst urban hea | Exarcheia, Athens, Greece | OK |
| 3 | heat_health | is paris ile-de-france going to be uninhabitable i | Saint-Denis, Paris, France | OK |
| 3 | new_question_type_natural_language | i'm worried about the climate where i live can you | Edinburgh, Scotland | OK |
| 3 | travel_safety | is iceland safe to drive in november ring road | Vík, Iceland | OK |
| 3 | travel_safety | avalanche risk chamonix france ski season | Chamonix, France | OK |
| 2 | energy_transition | wind capacity factor north sea netherlands | IJmuiden, Netherlands | OK |
| 2 | urban_planning | 15 minute city scoring barcelona eixample | Eixample, Barcelona, Spain | OK |

## East/SE Asia  (12 questions)

- Routing accuracy: **12/12 (100%)**
- Geocoder resolved: 12/12
- Returned >=1 signed fact: 12/12
- Avg latency: 24.89s

| Sev | Domain | Question | Place | Pass |
|---|---|---|---|---|
| 4 | air_quality | is hanoi air pollution worse than beijing now | Ba Dinh, Hanoi, Vietnam | OK |
| 4 | forest_carbon | oil palm expansion deforestation papua indonesia | Merauke, Papua, Indonesia | OK |
| 3 | esg_due_diligence | environmental impact zone around foxconn zhengzhou | Zhengzhou, Henan, China | OK |
| 3 | forest_carbon | peatland carbon stock central kalimantan | Palangka Raya, Indonesia | OK |
| 3 | insurance | typhoon parametric coverage philippines luzon | Tuguegarao, Cagayan, Philippin | OK |
| 3 | new_locations_under_observed | snow cover mongolia ulaanbaatar this winter | Ulaanbaatar, Mongolia | OK |
| 3 | new_question_type_natural_language | what should i be scared of climate-wise here | Manila, Philippines | OK |
| 3 | new_question_type_temporal | how has tree cover changed in singapore over the l | Bukit Timah, Singapore | OK |
| 3 | travel_safety | is it safe to visit bali during volcanic ash from  | Ubud, Bali, Indonesia | OK |
| 3 | travel_safety | thailand monsoon flood risk koh samui october | Koh Samui, Thailand | OK |
| 2 | energy_transition | offshore wind potential east coast vietnam | Phan Thiết, Vietnam | OK |
| 2 | urban_planning | how dense is kowloon hong kong actually | Mong Kok, Hong Kong | OK |

## Africa  (9 questions)

- Routing accuracy: **9/9 (100%)**
- Geocoder resolved: 9/9
- Returned >=1 signed fact: 9/9
- Avg latency: 43.42s

| Sev | Domain | Question | Place | Pass |
|---|---|---|---|---|
| 5 | food_security | horn of africa drought somalia food crisis | Baidoa, Somalia | OK |
| 4 | coastal | coastal erosion lagos nigeria victoria island | Victoria Island, Lagos, Nigeri | OK |
| 4 | esg_due_diligence | deforestation footprint cobalt mining katanga drc | Kolwezi, Democratic Republic o | OK |
| 4 | forest_carbon | congo basin forest loss kisangani | Kisangani, Democratic Republic | OK |
| 3 | forest_carbon | great green wall progress sahel senegal | Linguère, Senegal | OK |
| 3 | new_locations_under_observed | vegetation health madagascar dry south | Toliara, Madagascar | OK |
| 3 | new_locations_under_observed | sahel rainfall trends timbuktu mali | Timbuktu, Mali | OK |
| 3 | new_locations_under_observed | oil spill recovery delta niger | Bonny, Rivers State, Nigeria | OK |
| 3 | new_question_type_temporal | how is built up area expanding around addis ababa | Bole, Addis Ababa, Ethiopia | OK |

## Middle East  (8 questions)

- Routing accuracy: **8/8 (100%)**
- Geocoder resolved: 8/8
- Returned >=1 signed fact: 8/8
- Avg latency: 22.73s

| Sev | Domain | Question | Place | Pass |
|---|---|---|---|---|
| 5 | flooding | is dubai still flooded after the april rains | Dubai, United Arab Emirates | OK |
| 5 | flooding | derna libya dam collapse flood damage | Derna, Libya | OK |
| 5 | heat_health | hajj heatstroke risk mecca | Mecca, Saudi Arabia | OK |
| 4 | coastal | sea rise threat to alexandria egypt | Alexandria, Egypt | OK |
| 4 | flooding | is khartoum at risk from nile flooding this year | Khartoum, Sudan | OK |
| 4 | new_question_type_temporal | trend of summer maximum temperatures in baghdad ov | Baghdad, Iraq | OK |
| 3 | new_question_type_natural_language | is my hometown going to be uninhabitable | Basra, Iraq | OK |
| 3 | travel_safety | sahara dust haboob risk marrakech | Marrakech, Morocco | OK |

## South America  (7 questions)

- Routing accuracy: **7/7 (100%)**
- Geocoder resolved: 7/7
- Returned >=1 signed fact: 7/7
- Avg latency: 29.77s

| Sev | Domain | Question | Place | Pass |
|---|---|---|---|---|
| 4 | flooding | porto alegre brazil flood recovery 2024 | Porto Alegre, Brazil | OK |
| 4 | forest_carbon | is the amazon nearing the tipping point in para st | Altamira, Pará, Brazil | OK |
| 4 | wildfire | burn scar from valparaiso chile fire 2024 | Valparaíso, Chile | OK |
| 3 | esg_due_diligence | water stress around lithium mine salar de atacama | Salar de Atacama, Chile | OK |
| 3 | food_security | coffee rust risk minas gerais brazil | Varginha, Minas Gerais, Brazil | OK |
| 3 | water_security | dam levels itaipu paraguay brazil | Itaipu Dam, Paraguay | OK |
| 2 | energy_transition | utility solar farm siting potential atacama chile | Calama, Chile | OK |

## Other  (6 questions)

- Routing accuracy: **4/6 (66%)**
- Geocoder resolved: 6/6
- Returned >=1 signed fact: 6/6
- Avg latency: 28.87s

| Sev | Domain | Question | Place | Pass |
|---|---|---|---|---|
| 5 | wildfire | wildfire near maui kula upcountry | Kula, Maui, Hawaii | OK |
| 4 | water_security | aral sea recovery uzbekistan kazakhstan | Muynak, Uzbekistan | FAIL |
| 3 | air_quality | saharan dust storm air quality canary islands | Las Palmas, Canary Islands | OK |
| 3 | new_locations_under_observed | earthquake vulnerability and topography port-au-pr | Pétion-Ville, Port-au-Prince,  | OK |
| 3 | new_locations_under_observed | dust bowl risk aralkum desert | Aralkum Desert, Uzbekistan | OK |
| 1 | out_of_scope_check | what is the meaning of life | Earth | FAIL |

## Oceania  (4 questions)

- Routing accuracy: **4/4 (100%)**
- Geocoder resolved: 4/4
- Returned >=1 signed fact: 4/4
- Avg latency: 26.43s

| Sev | Domain | Question | Place | Pass |
|---|---|---|---|---|
| 5 | coastal | will tuvalu still exist in 2050 | Funafuti, Tuvalu | OK |
| 4 | wildfire | bushfire risk blue mountains nsw this summer | Katoomba, New South Wales, Aus | OK |
| 3 | insurance | drought index payout for cotton farmers australia | Wee Waa, New South Wales, Aust | OK |
| 3 | new_locations_under_observed | flood risk in pacific island nation kiribati | South Tarawa, Kiribati | OK |

## Polar  (2 questions)

- Routing accuracy: **2/2 (100%)**
- Geocoder resolved: 2/2
- Returned >=1 signed fact: 2/2
- Avg latency: 17.20s

| Sev | Domain | Question | Place | Pass |
|---|---|---|---|---|
| 4 | glacier_polar | is thwaites glacier collapse imminent | Thwaites Glacier, Antarctica | OK |
| 4 | glacier_polar | greenland ice sheet melt jakobshavn | Ilulissat, Greenland | OK |
