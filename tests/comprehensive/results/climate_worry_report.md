# Climate worry report — what emem says about the questions that matter most

Filtered to severity >= 4. These are the questions where a wrong or absent answer has real human cost: home-purchase decisions, evacuation, food security, hajj heatstroke, low-lying island survival.

Sample size: **52** high-severity questions.

Each row shows: the consumer question, the place emem resolved (cell64), the topics it routed to, how many signed facts came back, and the routing verdict. Cells that returned facts are *verifiable* — the responder signed the fact CIDs and any client can re-fetch the same bytes.

| Sev | Domain | Question | Place resolved (cell64) | Topics matched | Facts | Pass |
|---|---|---|---|---|---|---|
| 5 | coastal | will tuvalu still exist in 2050 | Funafuti (town), Tuvalu `defi.zb39f.cUnA.zdb82` | `weather_now`, `flood_risk_composite`, `esg` | 20 | OK |
| 5 | coastal | sinking lands and sea rise male maldives | މާލެ (city), ދިވެހިރާއްޖެ `defi.zb42f.vIpe.delo` | `elevation_global_topobathy`, `topography`, `flood_water_event_window` | 16 | OK |
| 5 | flooding | is dubai still flooded after the april rains | Dubai, UAE `defi.zb51e.zc669.zd36f` | `flood_history_long_term`, `flood_risk_composite`, `flood_water_event_window` | 19 | OK |
| 5 | flooding | valencia spain floods october 2024 inundation extent | València (city), Comunitat Valencia `defi.zb5c1.dugI.zeedf` | `flood_history_long_term`, `flood_risk_composite`, `flood_water_event_window` | 19 | OK |
| 5 | flooding | derna libya dam collapse flood damage | درنة (city), درنة, ليبيا `defi.zb574.zbaa4.bIse` | `flood_water_event_window`, `flood_history_long_term`, `parametric_insurance` | 19 | OK |
| 5 | food_security | horn of africa drought somalia food crisis | Baydhabo بيدوا (city), Koonfur Galb `defi.zb423.tAqU.zc24e` | `parametric_insurance`, `agriculture`, `weather_now` | 28 | OK |
| 5 | heat_health | how dangerous is the heat dome in seville this week | Sevilla (city), Andalucía, España `defi.zb5a9.quni.zef25` | `vegetation_condition`, `public_health`, `parametric_insurance` | 32 | OK |
| 5 | heat_health | heatwave delhi may 2026 health risk for kids | Connaught Place (suburb), Delhi, In `defi.zb545.zc2ed.zba56` | `public_health`, `parametric_insurance`, `weather_now` | 29 | OK |
| 5 | heat_health | hajj heatstroke risk mecca | مكة (province), منطقة مكة المكرمة,  `defi.zb4f3.zb8a7.dAcu` | `parametric_insurance`, `public_health`, `fire_burn_severity` | 31 | OK |
| 5 | wildfire | is my home in the palisades likely to burn this fire se | Pacific Palisades (suburb), Los Ang `defi.zb583.qeza.zedba` | `fire_burn_severity`, `real_estate`, `flood_risk_composite` | 26 | OK |
| 5 | wildfire | wildfire near maui kula upcountry | Kula (village), Hawaii, United Stat `defi.zb4ec.wEvu.jAhA` | `fire_burn_severity`, `flood_water_event_window`, `parametric_insurance` | 32 | OK |
| 4 | air_quality | is the air safe to walk outside in lahore today | گلبرگ (suburb), لاہور کینٹ, پنجاب,  `defi.zb566.vIpe.jIqa` | `public_health`, `urban_livability`, `weather_now` | 23 | OK |
| 4 | air_quality | smog in delhi gurgaon noida how bad is it | Sector 18 (suburb), Noida, Uttar Pr `defi.zb545.depA.zbf0c` | `public_health`, `weather_now`, `esg` | 29 | OK |
| 4 | air_quality | is hanoi air pollution worse than beijing now | Quảng trường Ba Đình (square), Hà N `defi.zb4ef.papO.zd0b4` | `public_health`, `weather_now`, `esg` | 30 | OK |
| 4 | climate_migration | is duluth minnesota actually a climate refuge | Duluth (city), Minnesota, United St `defi.zb614.mido.yAhO` | `weather_now`, `flood_water_event_window`, `parametric_insurance` | 24 | OK |
| 4 | climate_migration | is buffalo new york a good climate haven for retirement | Buffalo (city), New York, United St `defi.zb5e7.zf391.zfa2b` | `urban_livability`, `weather_now`, `esg` | 27 | OK |
| 4 | climate_migration | how livable will phoenix be in 2050 with extreme heat | Phoenix (city), Arizona, United Sta `defi.zb57c.wIma.dore` | `urban_livability`, `weather_now`, `public_health` | 23 | OK |
| 4 | coastal | coastal erosion lagos nigeria victoria island | Victoria Island (suburb), Itirin, L `defi.zb449.gUwu.yacA` | `flood_water_event_window`, `esg`, `built_up_human_geography` | 22 | OK |
| 4 | coastal | sea rise threat to alexandria egypt | الإسكندرية (city), الإسكندرية, مصر `defi.zb562.zfa25.mOgo` | `flood_water_event_window`, `flood_risk_composite`, `flood_history_long_term` | 19 | OK |
| 4 | esg_due_diligence | deforestation footprint cobalt mining katanga drc | Kolwezi (city), Lualaba, République `defi.zb386.daja.vuqI` | `esg`, `carbon_credits`, `vegetation_condition` | 27 | OK |
| 4 | flooding | how often does chennai velachery actually waterlog ever | Velachery (suburb), Chennai, Tamil  `defi.zb493.zaf6e.lesU` | `flood_water_event_window`, `flood_history_long_term`, `flood_risk_composite` | 19 | OK |
| 4 | flooding | is khartoum at risk from nile flooding this year | الخرطوم (city), الخرطوم, السودان `defi.zb4b1.dupI.zc8b3` | `flood_risk_composite`, `flood_history_long_term`, `flood_water_event_window` | 19 | OK |
| 4 | flooding | porto alegre brazil flood recovery 2024 | Porto Alegre (municipality), Rio Gr `defi.zb2aa.mitO.ze472` | `flood_history_long_term`, `flood_risk_composite`, `flood_water_event_window` | 19 | OK |
| 4 | food_security | how is the maize harvest looking in the us corn belt | Ames (city), Iowa, United States `defi.zb5de.hode.pelu` | `agriculture`, `vegetation_condition`, `soil_bare` | 33 | OK |
| 4 | food_security | groundwater stress on rice farms central valley califor | Fresno (city), California, United S `defi.zb5a2.bubi.zb475` | `soil_intelligence`, `agriculture`, `parametric_insurance` | 40 | OK |
| 4 | food_security | rice paddies inundated by floods in sindh pakistan | لاڑکانہ (city), سندھ, پاکستان `defi.zb539.vEba.fUlo` | `flood_history_long_term`, `flood_water_event_window`, `flood_risk_composite` | 19 | OK |
| 4 | forest_carbon | is the amazon nearing the tipping point in para state | Altamira (municipality), Pará, Bras `defi.zb3db.vUxa.zb7de` | `esg`, `parametric_insurance`, `carbon_credits` | 24 | OK |
| 4 | forest_carbon | oil palm expansion deforestation papua indonesia | Merauke (town), Maro, Papua Selatan `defi.zb39f.quyi.zf58e` | `esg`, `carbon_credits`, `vegetation_condition` | 28 | OK |
| 4 | forest_carbon | congo basin forest loss kisangani | Kisangani (city), Tshopo, Républiqu `defi.zb405.ze5e4.tado` | `esg`, `carbon_credits`, `vegetation_condition` | 25 | OK |
| 4 | glacier_polar | is thwaites glacier collapse imminent | Thwaites Glacier (glacier) `defi.zb0aa.ze90c.zff5e` | `flood_risk_composite`, `flood_water_event_window`, `esg` | 18 | OK |
| 4 | glacier_polar | how fast is the gangotri glacier retreating | Gangotri Glacier (glacier), Uttarak `defi.zb55e.zbc6e.dara` | `weather_now`, `topography`, `snow` | 20 | OK |
| 4 | glacier_polar | swiss alps glacier loss aletsch | Grosser Aletschgletscher (glacier), `defi.zb610.vecA.rEco` | `snow`, `topography`, `parametric_insurance` | 20 | OK |
| 4 | glacier_polar | greenland ice sheet melt jakobshavn | Ilulissat (city), Kalaallit Nunaat `defi.zb713.wEpO.zea71` | `snow`, `weather_now`, `carbon_credits` | 18 | OK |
| 4 | heat_health | how hot are nights in karachi getting these days | کراچی (city), سندھ, پاکستان `defi.zb51a.zca6b.zea2f` | `weather_now`, `urban_livability`, `public_health` | 30 | OK |
| 4 | insurance | hurricane parametric trigger probability gulf coast lou | Lafayette (city), Louisiana, United `defi.zb557.ze80f.zadE` | `parametric_insurance`, `weather_now`, `analytics` | 20 | OK |
| 4 | insurance | why is home insurance unaffordable in cape coral florid | Cape Coral (city), Florida, United  `defi.zb52e.jUpU.ruxu` | `real_estate`, `flood_risk_composite`, `parametric_insurance` | 22 | OK |
| 4 | insurance | insurer non-renewals santa rosa california wildfire ris | Santa Rosa (city), California, Unit `defi.zb5b5.puka.hImU` | `fire_burn_severity`, `real_estate`, `parametric_insurance` | 25 | OK |
| 4 | new_question_type_temporal | trend of summer maximum temperatures in baghdad over 20 | بغداد (city), بغداد, العراق `defi.zb57a.zf327.ze41c` | `weather_now`, `public_health`, `parametric_insurance` | 26 | OK |
| 4 | real_estate | should i buy a flat in lower parel mumbai or is it floo | Lower Parel (suburb), Maharashtra,  `defi.zb4d8.jEbO.zf296` | `flood_risk_composite`, `real_estate`, `flood_history_long_term` | 15 | OK |
| 4 | real_estate | is gurgaon sector 65 safe from waterlogging in monsoon | Sector 65 (suburb), Gurgaon, Haryān `defi.zb543.havU.zb385` | `flood_water_event_window`, `flood_risk_composite`, `flood_history_long_term` | 19 | OK |
| 4 | real_estate | thinking of buying a beach house in tampa florida how r | Tampa (city), Florida, United State `defi.zb53d.zf391.nEke` | `flood_risk_composite`, `real_estate`, `parametric_insurance` | 19 | OK |
| 4 | real_estate | is it dumb to buy a house in paradise california after  | Paradise (town), California, United `defi.zb5c4.pIcE.pOcu` | `flood_risk_composite`, `real_estate`, `fire_burn_severity` | 25 | OK |
| 4 | real_estate | sea level rise threat to flats in bandra reclamation | Bandra Reclamation (neighbourhood), `defi.zb4d8.zc2ac.zf2b1` | `flood_risk_composite`, `flood_water_event_window`, `weather_now` | 32 | OK |
| 4 | water_security | is bengaluru going to run out of water like 2024 again | Bellanduru (suburb), Karnataka, Ind `defi.zb493.fodI.zcf2a` | `flood_water_event_window`, `flood_risk_composite`, `weather_now` | 19 | OK |
| 4 | water_security | reservoir levels barcelona drought | Vilanova de Sau (village), Cataluny `defi.zb5dd.luqa.rigO` | `flood_water_event_window`, `weather_now`, `flood_history_long_term` | 19 | OK |
| 4 | water_security | is the great salt lake going to disappear | Antelope Island (island), Utah, Uni `defi.zb5d1.zff4c.cEcu` | `flood_water_event_window`, `flood_risk_composite`, `weather_now` | 19 | OK |
| 4 | water_security | aral sea recovery uzbekistan kazakhstan | Moynaq qoltıǵı kóli (wetland), Qara `defi.zb5f2.cEku.sOyI` | `flood_water_event_window`, `esg`, `carbon_credits` | 25 | FAIL |
| 4 | wildfire | how bad is wildfire smoke in toronto today from quebec  | Toronto, Canada `defi.zb5f0.zad11.ze32e` | `public_health`, `fire_burn_severity`, `weather_now` | 24 | OK |
| 4 | wildfire | smoke from canadian wildfires new york city | Manhattan (suburb), New York, New Y `defi.zb5cf.zbc12.zd8d6` | `fire_burn_severity`, `public_health`, `parametric_insurance` | 33 | OK |
| 4 | wildfire | bushfire risk blue mountains nsw this summer | Katoomba (town), New South Wales, A `defi.zb280.qOxE.zb8df` | `parametric_insurance`, `flood_risk_composite`, `fire_burn_severity` | 24 | OK |
| 4 | wildfire | how much of rhodes greece burned in the 2023 fires | Ρόδος (island), Αποκεντρωμένη Διοίκ `defi.zb59b.wAta.zf6a4` | `fire_burn_severity`, `public_health`, `parametric_insurance` | 29 | OK |
| 4 | wildfire | burn scar from valparaiso chile fire 2024 | Valparaíso (city), Región de Valpar `defi.zb288.botI.lonO` | `fire_burn_severity`, `parametric_insurance`, `optical_raw_reflectance` | 36 | OK |

## High-severity routing misses (fix list)

Each bullet is one consumer who got the wrong primitive routed for a high-stakes question.

### water_security

- **aral sea recovery uzbekistan kazakhstan**  
  place: `Muynak, Uzbekistan`, expected one of `['flood_history_long_term', 'topography', 'vegetation_condition']`, got `['flood_water_event_window', 'esg', 'carbon_credits', 'weather_now', 'parametric_insurance']`, out_of_scope=False
