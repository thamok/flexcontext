export type RuntimeConfig = {
  authenticationPrefix: string;
  retryCount: number;
};

export const runtimeConfig: RuntimeConfig = {
  authenticationPrefix: "Bearer ",
  retryCount: 3,
};
