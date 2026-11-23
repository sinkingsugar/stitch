---
name: google-flights
description: Search Google Flights for flights between two airports on given dates, and find the cheapest dates over a range, with the google_flights_search and google_flights_cheap_dates tools.
---

## Google Flights

Two tools come with this skill, written in Otter and run by the node; they
read the public Google Flights website (no API key). They are lazy: not in
your tool list, but called with `Tool.Call` in `otter_run` (the otter
skill), their parameters given below when you read this with `read_skill`.
One search, or several dates in one run:

```otter
{from: "LHR" to: "JFK" date: "2026-12-01"} | Tool.Call("google_flights_search")
```

```otter
["2026-12-01" "2026-12-08"] = dates
dates | Map(date {
  {from: "LHR" to: "JFK" date: date} | Tool.Call("google_flights_search") = found
  f"## {date}\n{found}\n"
}) | String.Join
```

Each call is approved as the tool is when called directly (they ask, unless
`[otter] allow` or an "always" answer allows them); the run itself does not
ask for them.

- `google_flights_search`: one-way or round-trip flights between two
  airports on given dates: prices, airlines, flight numbers, times,
  durations, stops, layovers and a booking link. Round-trip prices are
  totals, and the options listed are the outbound legs.
- `google_flights_cheap_dates`: the cheapest dates over a range of up to 45
  days, one real search per date (about half a second a date), one-way or
  for a trip of a fixed number of days.

How to use them well:

- Airports are IATA codes (`LHR`, `JFK`, `KIX`). For a city with several
  airports, search the main ones, or ask which the user prefers.
- Dates are `YYYY-MM-DD`, in the future. When the user is flexible, scan
  with `google_flights_cheap_dates` first, then search the best dates for
  the flights themselves.
- Prices are in `currency` (USD unless asked): use the user's when you know
  it.
- Google sometimes returns pages without flights (rate limiting): the tools
  retry, then say so. Wait a minute before trying again; do not loop.
- Give the user the booking link from the result; you cannot book.

Their scripts are in `tools/` (read them with `read_skill`); they are a
port of edge talk's tool, kept with Otter's example
(`otter/examples/tools/flights`).
