// SPDX-License-Identifier: AGPL-3.0-or-later
//
// Imported first by main.ts so that it runs before any other module of the
// app: the page's [[id]] markers become words before anything reads them.

import { fillPage } from "./strings";

fillPage(globalThis.document);
