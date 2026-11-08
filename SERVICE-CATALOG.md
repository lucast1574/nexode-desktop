# Public service catalogue

Orbit's curated catalogue lives in `src/ecosystem.rs`. It helps users discover
relevant products and prepares an integration plan; a listing does not mean the
product is already connected, included in a Nexode plan, or accessible with a
Nexode API key. Each service retains its own authentication and billing.

## Additions verified on 2026-11-07

The following first-party public pages returned HTTP 200 during verification.
Descriptions below reflect their published capabilities, not an end-to-end test
of paid features or authenticated APIs. Private deployment inventory was used
only to identify candidates; no infrastructure details or credentials belong in
this catalogue.

| Product | Public evidence | Scope included in Orbit |
| --- | --- | --- |
| API Sunat | [Documentation](https://docs.apisunat.net), [quick start](https://docs.apisunat.net/inicio-rapido), [authentication](https://docs.apisunat.net/autenticacion) | Electronic invoices, receipts, notes and shipping guides. The documentation is the catalogue destination because it directly explains integration; landing-page availability is not assumed. |
| HeyBunny | [Product site](https://heybunny.si) | AI agents working with a user's tools, requesting approval when needed. No specific provider compatibility, uptime or cross-product access is promised. |
| Wazend Calls | [Public application](https://calls.wazend.net) | Incoming/outgoing WhatsApp calls and call history. The public application currently identifies itself as **ArendCalls**. Its delivered interface includes WhatsApp account pairing, incoming-call controls and call-history text. |
| Zend CRM | [Product site](https://zend.chat) | Sales management, WhatsApp conversations, workflows and AI agents, as described by the site's title and metadata. |
| Cerebrads | [Product site](https://cerebrads.lat) | Advertising campaign analysis, creatives and sales strategies using AI. Prices are deliberately not copied into Orbit; users must check current terms. |

For Wazend Calls, the inspected public application asset was
[`index-CJjHpm0G.js`](https://calls.wazend.net/assets/index-CJjHpm0G.js).
Its visible interface strings include WhatsApp account identification, call
history and controls for incoming calls. This verifies the catalogue's limited
description; it does not establish a supported external API contract. Read the
current product documentation before implementing an integration.

Repository metadata searches under the expected publisher did not expose the
candidate repositories to the available GitHub identity. This is not evidence
that those repositories do not exist. The additions above rely on public product
pages, not assumptions about private code.

## Existing entries and exclusions

The existing Nexode, Nexode Shield, Mapora, Vitruko, Wazend, Harkai, The Date,
Olva API and Shalom API entries are retained. This update does not expand their
existing capability claims.

The legacy EnviosShalom Shopify endpoint is excluded: it returned HTTP 410 during
the inventory review. This is separate from the existing Shalom API offering.
The Trustpilot API candidate is also excluded because a public product address
and supported integration contract were not verified. Internal-only endpoints
are never published as customer-facing service links.

Availability and product capabilities can change. Before any integration,
Orbit must inspect current public documentation, establish the user's access,
and obtain authorization for the concrete deployment or purchase. Catalogue
matching alone never sends user data to another product.
