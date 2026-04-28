import { Coordinates, distanceKm } from "./geotessera.js";

export type Place = {
  name: string;
  country: string;
  coordinates: Coordinates;
  aliases: string[];
};

export const PLACES: Place[] = [
  {
    name: "Bengaluru, Karnataka",
    country: "IN",
    coordinates: { lat: 12.9716, lng: 77.5946 },
    aliases: ["bangalore", "bengaluru", "karnataka"]
  },
  {
    name: "India Gate, New Delhi",
    country: "IN",
    coordinates: { lat: 28.6129, lng: 77.2295 },
    aliases: ["india gate", "new delhi", "delhi"]
  },
  {
    name: "Gateway of India, Mumbai",
    country: "IN",
    coordinates: { lat: 18.922, lng: 72.8347 },
    aliases: ["gateway of india", "mumbai", "bombay"]
  },
  {
    name: "Times Square, New York",
    country: "US",
    coordinates: { lat: 40.758, lng: -73.9855 },
    aliases: ["times square", "new york", "nyc"]
  },
  {
    name: "Golden Gate Bridge, San Francisco",
    country: "US",
    coordinates: { lat: 37.8199, lng: -122.4783 },
    aliases: ["golden gate", "san francisco", "sf"]
  },
  {
    name: "White House, Washington DC",
    country: "US",
    coordinates: { lat: 38.8977, lng: -77.0365 },
    aliases: ["white house", "washington dc", "washington"]
  },
  {
    name: "Eiffel Tower, Paris",
    country: "FR",
    coordinates: { lat: 48.8584, lng: 2.2945 },
    aliases: ["eiffel tower", "paris"]
  },
  {
    name: "Shibuya Crossing, Tokyo",
    country: "JP",
    coordinates: { lat: 35.6595, lng: 139.7005 },
    aliases: ["shibuya", "tokyo"]
  },
  {
    name: "Burj Khalifa, Dubai",
    country: "AE",
    coordinates: { lat: 25.1972, lng: 55.2744 },
    aliases: ["burj khalifa", "dubai"]
  },
  {
    name: "Sydney Opera House, Sydney",
    country: "AU",
    coordinates: { lat: -33.8568, lng: 151.2153 },
    aliases: ["sydney opera house", "sydney"]
  },
  {
    name: "Christ the Redeemer, Rio de Janeiro",
    country: "BR",
    coordinates: { lat: -22.9519, lng: -43.2105 },
    aliases: ["christ the redeemer", "rio", "rio de janeiro"]
  },
  {
    name: "Cape Town Waterfront",
    country: "ZA",
    coordinates: { lat: -33.9036, lng: 18.4219 },
    aliases: ["cape town", "waterfront"]
  }
];

export function resolveNearestPlace(coordinates: Coordinates): {
  country: string;
  nearestPlace: string;
} {
  const nearest = nearestPlace(coordinates);
  return {
    country: nearest.place.country,
    nearestPlace: nearest.place.name
  };
}

export function nearestPlace(coordinates: Coordinates): {
  place: Place;
  distanceKm: number;
} {
  return PLACES.map((place) => ({
    place,
    distanceKm: distanceKm(coordinates, place.coordinates)
  })).sort((left, right) => left.distanceKm - right.distanceKm)[0];
}

export function searchPlaces(query: string, limit = 5): Place[] {
  const normalized = query.trim().toLowerCase();

  if (!normalized) {
    return [];
  }

  return PLACES.map((place) => {
    const haystack = [place.name, ...place.aliases].join(" ").toLowerCase();
    const starts = [place.name, ...place.aliases].some((value) =>
      value.toLowerCase().startsWith(normalized)
    );
    const includes = haystack.includes(normalized);
    return {
      place,
      score: starts ? 0 : includes ? 1 : 2
    };
  })
    .filter((result) => result.score < 2)
    .sort((left, right) => left.score - right.score || left.place.name.localeCompare(right.place.name))
    .slice(0, limit)
    .map((result) => result.place);
}
