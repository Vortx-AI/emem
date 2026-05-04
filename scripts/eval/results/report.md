# emem.dev routing + answer evaluation

- Questions: 51
- Routing-correct (any expected topic in matched): **46/51 (90%)**
- HTTP 200: 50/51
- Returned ≥1 fact: 50/51
- Average end-to-end latency: 41.52s

## By domain

| Domain | Pass | Total | Avg latency | Avg facts |
|---|---|---|---|---|
| agriculture | 6 | 6 | 29.7s | 3.0 |
| calamity | 6 | 6 | 58.0s | 3.0 |
| carbon | 5 | 5 | 33.9s | 3.0 |
| climate | 4 | 5 | 33.3s | 3.0 |
| esg | 5 | 5 | 36.5s | 3.0 |
| forest | 5 | 5 | 58.7s | 3.0 |
| health | 5 | 5 | 31.8s | 3.0 |
| property | 3 | 5 | 50.3s | 2.4 |
| urban | 4 | 4 | 19.6s | 3.0 |
| water | 3 | 5 | 58.2s | 3.0 |

## Per-question results

| ID | Domain | Question | Routed via | Top match | Pass? | Facts | Latency |
|---|---|---|---|---|---|---|---|
| 1 | health | what is the air quality in new delhi today | ort | `public_health` (0.548) | ✓ | 3 | 15.0s |
| 2 | health | heat stroke risk in phoenix this week | ort | `parametric_insurance` (0.702) | ✓ | 3 | 29.0s |
| 3 | health | pm2.5 pollution exposure beijing | ort | `public_health` (0.156) | ✓ | 3 | 61.7s |
| 4 | health | wildfire smoke risk los angeles | ort | `fire_burn_severity` (0.669) | ✓ | 3 | 28.2s |
| 5 | health | pm2.5 levels lahore pakistan | ort | `public_health` (0.179) | ✓ | 3 | 25.2s |
| 6 | property | flood risk for a property in houston | ort | `flood_risk_composite` (0.278) | ✓ | 3 | 79.1s |
| 7 | property | walkability score for soho manhattan | ort | `urban_livability` (0.741) | ✓ | 3 | 23.1s |
| 8 | property | land subsidence in jakarta | ort | `built_up_human_geography` (0.679) | ✗ | 3 | 35.4s |
| 9 | property | coastal erosion miami beach | None | — | ✗ | 0 | 90.1s |
| 10 | property | building density tokyo shibuya | ort | `built_up_human_geography` (0.705) | ✓ | 3 | 23.7s |
| 11 | calamity | recent flood damage pakistan sindh | ort | `flood_history_long_term` (0.755) | ✓ | 3 | 80.7s |
| 12 | calamity | active wildfire risk northern california | ort | `fire_burn_severity` (0.659) | ✓ | 3 | 59.4s |
| 13 | calamity | drought severity cape town | ort | `parametric_insurance` (0.705) | ✓ | 3 | 69.6s |
| 14 | calamity | cyclone path bangladesh | ort | `parametric_insurance` (0.684) | ✓ | 3 | 34.7s |
| 15 | calamity | burned area near athens greece | ort | `fire_burn_severity` (0.752) | ✓ | 3 | 26.5s |
| 16 | calamity | storm surge risk new orleans | ort | `flood_risk_composite` (0.681) | ✓ | 3 | 77.0s |
| 17 | carbon | carbon stock in the amazon basin | ort | `carbon_credits` (0.763) | ✓ | 3 | 30.8s |
| 18 | carbon | co2 emissions hotspots over shanghai | ort | `analytics` (0.194) | ✓ | 3 | 40.3s |
| 19 | carbon | tree cover loss indonesia 2023 | ort | `esg` (0.716) | ✓ | 3 | 34.6s |
| 20 | carbon | soil organic carbon great plains | ort | `soil_intelligence` (0.594) | ✓ | 3 | 40.5s |
| 21 | carbon | mangrove carbon density sundarbans | ort | `carbon_credits` (0.719) | ✓ | 3 | 23.2s |
| 22 | esg | biodiversity index western ghats india | ort | `esg` (0.316) | ✓ | 3 | 36.2s |
| 23 | esg | environmental risk textile factory tirupur | ort | `esg` (0.702) | ✓ | 3 | 27.1s |
| 24 | esg | land use change near tesla gigafactory berlin | ort | `built_up_human_geography` (0.705) | ✓ | 3 | 42.8s |
| 25 | esg | water stress for cotton farms in punjab | ort | `parametric_insurance` (0.734) | ✓ | 3 | 58.5s |
| 26 | esg | emissions impact zone around mumbai port | ort | `public_health` (0.707) | ✓ | 3 | 17.9s |
| 27 | agriculture | wheat crop health indiana | ort | `vegetation_condition` (0.44) | ✓ | 3 | 33.3s |
| 28 | agriculture | irrigation demand rice paddies vietnam | ort | `agriculture` (0.634) | ✓ | 3 | 46.4s |
| 29 | agriculture | soil organic carbon iowa | ort | `soil_intelligence` (0.792) | ✓ | 3 | 34.9s |
| 30 | agriculture | crop stress in punjab india | ort | `agriculture` (0.724) | ✓ | 3 | 10.8s |
| 31 | agriculture | ndvi of corn fields nebraska | ort | `vegetation_condition` (0.25) | ✓ | 3 | 14.6s |
| 32 | agriculture | olive grove vigor andalusia spain | ort | `agriculture` (0.685) | ✓ | 3 | 38.0s |
| 33 | forest | forest canopy cover borneo | ort | `esg` (0.714) | ✓ | 3 | 64.8s |
| 34 | forest | fire scars in yellowstone | ort | `fire_burn_severity` (0.781) | ✓ | 3 | 54.2s |
| 35 | forest | tropical deforestation rate brazil | ort | `esg` (0.679) | ✓ | 3 | 63.5s |
| 36 | forest | boreal forest health siberia | ort | `esg` (0.679) | ✓ | 3 | 47.1s |
| 37 | forest | reforestation progress ethiopia tigray | ort | `esg` (0.683) | ✓ | 3 | 63.7s |
| 38 | water | lake mead water level | ort | `flood_water_event_window` (0.756) | ✓ | 3 | 65.7s |
| 39 | water | groundwater depletion in punjab | ort | `soil_intelligence` (0.643) | ✗ | 3 | 83.7s |
| 40 | water | surface water in chad basin | ort | `flood_water_event_window` (0.694) | ✓ | 3 | 46.9s |
| 41 | water | coastal water turbidity off mumbai | ort | `flood_water_event_window` (0.653) | ✗ | 3 | 34.8s |
| 42 | water | river discharge ganges varanasi | ort | `flood_water_event_window` (0.643) | ✓ | 3 | 60.0s |
| 43 | urban | urban heat island phoenix | ort | `urban_livability` (0.68) | ✓ | 3 | 4.3s |
| 44 | urban | solar pv potential rooftops berlin | ort | `urban_livability` (0.637) | ✓ | 3 | 25.1s |
| 45 | urban | road density central lagos | ort | `built_up_human_geography` (0.706) | ✓ | 3 | 26.0s |
| 46 | urban | building footprints doha qatar | ort | `built_up_human_geography` (0.678) | ✓ | 3 | 23.0s |
| 47 | climate | temperature trend sahel | ort | `weather_now` (0.701) | ✓ | 3 | 30.4s |
| 48 | climate | precipitation deficit east africa | ort | `parametric_insurance` (0.687) | ✓ | 3 | 66.8s |
| 49 | climate | wind energy potential west texas | ort | `carbon_credits` (0.607) | ✗ | 3 | 31.5s |
| 50 | climate | snow cover trend hindu kush | ort | `snow` (0.37) | ✓ | 3 | 30.4s |
| 51 | climate | sea surface temperature off cape town | ort | `weather_now` (0.724) | ✓ | 3 | 7.5s |