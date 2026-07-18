const minimum = [20, 0, 0];
const current = process.versions.node.split(".").map((part) => Number.parseInt(part, 10));

const compareVersions = (left, right) => {
  for (let index = 0; index < right.length; index += 1) {
    const difference = left[index] - right[index];

    if (difference !== 0) {
      return difference;
    }
  }

  return 0;
};

const isSupported = compareVersions(current, minimum) >= 0;

if (!isSupported) {
  console.error(`Node.js ${minimum.join(".")} or newer is required. Current version: ${process.versions.node}`);
  process.exit(1);
}

console.log(`Node.js ${process.versions.node} OK`);
