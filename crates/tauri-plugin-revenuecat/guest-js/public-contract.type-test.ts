import type { Offering, Offerings } from "./index.js";

declare const offerings: Offerings;

const currentOfferingId: string | null | undefined = offerings.current_offering_id;
const currentOffering: Offering | null = offerings.current;
const legacyManualValue: Offerings = {
  all: {},
  current: null,
};
const normalizedWireValue: Offerings = {
  all: {},
  current_offering_id: null,
  current: null,
};

void currentOfferingId;
void currentOffering;
void legacyManualValue;
void normalizedWireValue;
