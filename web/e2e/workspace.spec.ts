import { expect, test } from "@playwright/test";
test("example checks, feature selection, downloads and project reopen", async ({
  page,
}, testInfo) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/");
  await page
    .getByRole("button", { name: "Synthetic topology tutorial" })
    .click();
  await expect(
    page.locator(".layer-name").filter({ hasText: "streets" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Run topology checks" }).click();
  await expect(page.locator("tbody tr")).not.toHaveCount(0);
  await page
    .getByRole("button", { name: "Inspect violation 1", exact: true })
    .click();
  await expect(
    page.getByRole("heading", { name: "Violation details" }),
  ).toBeVisible();
  await expect(page.getByText(/FID /).first()).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("workspace.png") });
  await page.getByRole("button", { name: "Export report" }).click();
  const jsonDownload = page.waitForEvent("download");
  await page.getByRole("menuitem", { name: "Complete JSON report" }).click();
  expect((await jsonDownload).suggestedFilename()).toBe("topology-report.json");
  await page.getByRole("button", { name: "Export report" }).click();
  const gpkg = page.waitForEvent("download");
  await page.getByRole("menuitem", { name: "GeoPackage" }).click();
  expect((await gpkg).suggestedFilename()).toBe("topology-report.gpkg");
  const projectDownload = page.waitForEvent("download");
  await page.getByRole("button", { name: "Save project" }).click();
  const project = await projectDownload;
  const path = await project.path();
  await page.locator("input[type=file]").nth(1).setInputFiles(path!);
  await expect(page.locator("tbody tr")).toHaveCount(0);
  await expect(
    page.locator(".layer-name").filter({ hasText: "streets" }),
  ).toBeVisible();
  expect(errors).toEqual([]);
});
test("upload, configure a rule, run, invalidate and save", async ({ page }) => {
  await page.goto("/");
  const data = {
    type: "FeatureCollection",
    features: [
      {
        type: "Feature",
        id: -12,
        properties: { name: "First" },
        geometry: { type: "Point", coordinates: [5, 52] },
      },
      {
        type: "Feature",
        id: 42,
        properties: { name: "Second" },
        geometry: { type: "Point", coordinates: [5, 52] },
      },
    ],
  };
  await page
    .locator("input[type=file]")
    .first()
    .setInputFiles({
      name: "points.geojson",
      mimeType: "application/geo+json",
      buffer: Buffer.from(JSON.stringify(data)),
    });
  await expect(page.getByText("2 features · point")).toBeVisible();
  await page.getByLabel("Rule to add").selectOption("point/must-not-overlap");
  await page.getByRole("button", { name: "Add rule" }).click();
  await page.getByRole("button", { name: "Run topology checks" }).click();
  await expect(page.locator("tbody tr")).toHaveCount(1);
  await expect(page.locator("tbody")).toContainText("-12");
  await page.getByLabel("Tolerance", { exact: true }).fill("0.1");
  await page.getByRole("button", { name: "Apply configuration" }).click();
  await expect(page.locator("tbody tr")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Export report" }),
  ).toBeDisabled();
});

test("display mode preserves validation results and returns to the working grid", async ({
  page,
}) => {
  await page.route("https://*.tile.openstreetmap.org/**", (route) =>
    route.abort(),
  );
  await page.goto("/");
  await page
    .getByRole("button", { name: "Synthetic topology tutorial" })
    .click();
  await page.getByRole("button", { name: "Run topology checks" }).click();
  await expect(page.locator("tbody tr")).toHaveCount(3);
  const workingSelection = page.waitForResponse((response) =>
    response.url().includes("/violations/0?display=false"),
  );
  await page
    .getByRole("button", { name: "Inspect violation 1", exact: true })
    .click();
  const original = (await (await workingSelection).json()).geometry;
  await expect(
    page.getByRole("heading", { name: "Violation details" }),
  ).toBeVisible();
  const displaySelection = page.waitForResponse((response) =>
    response.url().includes("/violations/0?display=true"),
  );
  await page.getByRole("button", { name: "Add basemap" }).click();
  expect((await (await displaySelection).json()).geometry).not.toEqual(
    original,
  );
  await expect(page.getByText("OPENSTREETMAP", { exact: true })).toBeVisible();
  await expect(page.locator("tbody tr")).toHaveCount(3);
  await expect(
    page.getByRole("heading", { name: "Violation details" }),
  ).toBeVisible();
  const gridSelection = page.waitForResponse((response) =>
    response.url().includes("/violations/0?display=false"),
  );
  await page.getByRole("button", { name: "Switch to grid" }).click();
  expect((await (await gridSelection).json()).geometry).toEqual(original);
  await expect(
    page.getByText("COORDINATE GRID", { exact: true }),
  ).toBeVisible();
  await expect(page.locator("tbody tr")).toHaveCount(3);
  await expect(
    page.getByText("Run revision 1", { exact: false }),
  ).toBeVisible();
});

test("importing another layer preserves pending rule edits", async ({
  page,
}) => {
  await page.goto("/");
  const upload = async (name: string) => {
    await page
      .locator("input[type=file]")
      .first()
      .setInputFiles({
        name,
        mimeType: "application/geo+json",
        buffer: Buffer.from(
          JSON.stringify({
            type: "FeatureCollection",
            features: [
              {
                type: "Feature",
                id: -12,
                properties: {},
                geometry: { type: "Point", coordinates: [5, 52] },
              },
              {
                type: "Feature",
                id: 42,
                properties: {},
                geometry: { type: "Point", coordinates: [5, 52] },
              },
            ],
          }),
        ),
      });
  };
  await upload("stops.geojson");
  await expect(page.locator(".layer-card")).toHaveCount(1);
  await page.getByLabel("Rule to add").selectOption("point/must-not-overlap");
  await page.getByRole("button", { name: "Add rule" }).click();
  await expect(page.locator(".rule-card")).toHaveCount(1);
  await upload("other-stops.geojson");
  await expect(page.locator(".layer-card")).toHaveCount(2);
  await expect(page.locator(".rule-card")).toHaveCount(1);
  const colors = await page
    .locator(".color-input")
    .evaluateAll((inputs) =>
      inputs.map((input) => (input as HTMLInputElement).value),
    );
  expect(new Set(colors).size).toBe(2);
  await page.getByRole("button", { name: "Run topology checks" }).click();
  await expect(page.locator("tbody tr")).toHaveCount(1);
});
