import { expect, test } from "@playwright/test";

const examples = [
  { title: "World cities", layers: 2 },
  { title: "Major rivers", layers: 2 },
  { title: "Countries and islands", layers: 1 },
  { title: "Lakes and reservoirs", layers: 1 },
  { title: "Country and city atlas", layers: 1 },
];
for (const example of examples) {
  test(`open and inspect ${example.title}`, async ({ page }, testInfo) => {
    const errors: string[] = [];
    let previewUrl = "";
    page.on("response", (response) => {
      if (response.url().includes("/features?bbox="))
        previewUrl = response.url();
    });
    page.on("pageerror", (e) => errors.push(e.message));
    await page.route("https://*.tile.openstreetmap.org/**", (route) =>
      route.abort(),
    );
    await page.goto("/");
    await page
      .getByRole("button", { name: "Explore examples", exact: true })
      .click();
    const dialog = page.getByRole("dialog");
    await expect(dialog.locator(".example-card")).toHaveCount(5);
    await expect(
      dialog.getByText("Natural Earth · Public domain").first(),
    ).toBeVisible();
    if (example.title === "World cities")
      await page.screenshot({ path: testInfo.outputPath("gallery.png") });
    await dialog
      .getByRole("button", { name: `Open ${example.title}`, exact: true })
      .click();
    await expect(dialog).not.toBeVisible();
    await expect(page.locator(".layer-card")).toHaveCount(example.layers);
    await expect(
      page.getByText("COORDINATE GRID", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("button", { name: "Export report" }),
    ).toBeDisabled();
    await page.locator(".attribution summary").first().click();
    await expect(page.locator(".attribution").first()).toContainText(
      "Public domain",
    );
    await expect(page.locator(".attribution").first()).toContainText(
      "ca96624a56bd078437bca8184e78163e5039ad19",
    );
    await page.getByRole("button", { name: "Run topology checks" }).click();
    await expect(
      page.getByRole("button", { name: "Export report" }),
    ).toBeEnabled();
    await expect(page.locator(".clean-result, tbody tr").first()).toBeVisible();
    if (await page.locator("tbody tr").count()) {
      await page
        .getByRole("button", { name: "Inspect violation 1", exact: true })
        .click();
      await expect(
        page.getByRole("heading", { name: "Violation details" }),
      ).toBeVisible();
    } else {
      await expect(
        page.getByText("All configured checks passed"),
      ).toBeVisible();
      const response = await page.request.get(previewUrl);
      const preview = await response.json();
      const bounds = new URL(previewUrl).searchParams
        .get("bbox")!
        .split(",")
        .map(Number);
      const coord = preview.features[0].geometry.coordinates[0][0];
      const map = page.locator(".map-target");
      const box = (await map.boundingBox())!;
      await map.click({
        position: {
          x: ((coord[0] - bounds[0]!) / (bounds[2]! - bounds[0]!)) * box.width,
          y: ((bounds[3]! - coord[1]) / (bounds[3]! - bounds[1]!)) * box.height,
        },
      });
      await expect(
        page.getByRole("heading", { name: "Feature details" }),
      ).toBeVisible();
    }
    if (example.title === "Country and city atlas") {
      const download = page.waitForEvent("download");
      await page.getByRole("button", { name: "Save project" }).click();
      const project = await download;
      await page
        .locator("input[type=file]")
        .nth(1)
        .setInputFiles((await project.path())!);
      await expect(page.locator(".layer-card")).toHaveCount(1);
      await expect(
        page.getByRole("button", { name: "Export report" }),
      ).toBeDisabled();
      await page.locator(".attribution summary").first().click();
      await expect(page.locator(".attribution").first()).toContainText(
        "One GeometryCollection per country",
      );
    }
    await page.screenshot({ path: testInfo.outputPath("example.png") });
    expect(errors).toEqual([]);
  });
}

test("failed example loading preserves the current workspace", async ({
  page,
}) => {
  await page.goto("/");
  await page
    .getByRole("button", { name: "Synthetic topology tutorial", exact: true })
    .click();
  await expect(page.locator(".layer-name")).toContainText("streets");
  await page.getByRole("button", { name: "Examples", exact: true }).click();
  await page.route("**/api/examples/world-cities", (route) =>
    route.fulfill({
      status: 400,
      contentType: "application/json",
      body: JSON.stringify({ error: "Example import failed" }),
    }),
  );
  await page
    .getByRole("button", { name: "Open World cities", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await expect(page.getByRole("dialog").getByRole("alert")).toContainText(
    "Example import failed",
  );
  await page.getByRole("button", { name: "Close examples" }).click();
  await expect(page.locator(".layer-name")).toContainText("streets");
});
